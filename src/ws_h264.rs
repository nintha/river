use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use async_tungstenite::tungstenite::Message;
use crossbeam_utils::atomic::AtomicCell;
use futures::sink::SinkExt;
use futures::StreamExt;
use smol::net::{SocketAddr, TcpListener, TcpStream};

use crate::protocol::h264::Nalu;
use crate::rtmp_server::{eventbus_map, video_header_map};
use crate::protocol::rtmp::{ChunkMessageType, RtmpMessage};
use smol::channel::Receiver;
use smol::stream::{Stream};
use smol::stream;

#[allow(unused)]
pub(crate) async fn run_server(addr: &str) -> anyhow::Result<()> {
    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    log::info!("Websocket Listening on: {}", addr);

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        smol::spawn(handle_connection(stream, addr)).detach();
    }

    Ok(())
}


async fn handle_connection(raw_stream: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
    log::info!("Incoming TCP connection from: {}", addr);

    let uri = AtomicCell::default();
    let callback = |req: &Request, res: Response| -> Result<Response, ErrorResponse>{
        uri.store(req.uri().clone());
        Ok(res)
    };

    let ws_stream = async_tungstenite::accept_hdr_async(raw_stream, callback).await?;
    let (mut outgoing, _incoming) = ws_stream.split();

    let uri = uri.take();
    let stream_name = uri.path().strip_prefix("/websocket/")
        .ok_or(anyhow::anyhow!("invalid uri path"))?;
    log::info!("WebSocket connection established: {}, stream_name={}", addr, stream_name);

    // send video header
    if let Some(header) = video_header_map().get(stream_name) {
        for nalu in Nalu::from_rtmp_message(&header) {
            outgoing.send(Message::binary(nalu.as_ref())).await?;
        }
    }

    if let Some(el) = eventbus_map().get(stream_name) {
        let rx = el.register_receiver();
        std::mem::drop(el);

        let rx = rtmp_rx_into_nalu_rx(rx);
        futures::pin_mut!(rx);
        while let Some(nalu) = StreamExt::next(&mut rx).await{
            outgoing.send(Message::binary(nalu.as_ref())).await?;
        }
    }
    log::info!("WebSocket disconnected: {}, stream_name={}", addr, stream_name);
    Ok(())
}

// 把RMTP流转换城NALU流，并保证首帧为关键帧
fn rtmp_rx_into_nalu_rx(rx: Receiver<RtmpMessage>) -> impl Stream<Item=Nalu> {
    stream::unfold((rx, false), |(rx, first_key_frame)| async move {
        while let Ok(msg) = rx.recv().await {
            if msg.header.message_type != ChunkMessageType::VideoMessage {
                continue;
            }

            let nalus = Nalu::from_rtmp_message(&msg);
            if first_key_frame {
                return Some((stream::iter(nalus), (rx, first_key_frame)));
            }

            let nalus = nalus.into_iter().skip_while(|nalu| !nalu.is_key_frame).collect::<Vec<Nalu>>();

            if nalus.is_empty(){
                continue;
            }
            return Some((stream::iter(nalus), (rx, true)));
        }
        None
    }).flatten()
}

