use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use async_tungstenite::tungstenite::Message;
use crossbeam_utils::atomic::AtomicCell;
use futures::sink::SinkExt;
use futures::StreamExt;
use smol::net::{SocketAddr, TcpListener, TcpStream};

use crate::protocol::h264::Nalu;
use crate::rtmp_server::{eventbus_map, video_header_map};
use crate::protocol::rtmp::ChunkMessageType;

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

        // 第一个关键帧是否出现
        let mut first_key_frame = false;
        while let Ok(msg) = rx.recv().await {
            if msg.header.message_type != ChunkMessageType::VideoMessage {
                continue;
            }

            for nalu in Nalu::from_rtmp_message(&msg) {
                if !first_key_frame {
                    if nalu.is_key_frame {
                        first_key_frame = true;
                    } else {
                        continue;
                    }
                }
                outgoing.send(Message::binary(nalu.as_ref())).await?;
            }
        }
    }
    log::info!("WebSocket disconnected: {}, stream_name={}", addr, stream_name);
    Ok(())
}

