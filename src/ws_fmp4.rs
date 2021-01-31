use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use async_tungstenite::tungstenite::Message;
use crossbeam_utils::atomic::AtomicCell;
use futures::sink::SinkExt;
use futures::StreamExt;
use smol::net::{SocketAddr, TcpListener, TcpStream};

use crate::protocol::h264::Nalu;
use crate::rtmp_server::{eventbus_map, video_header_map, meta_data_map};
use crate::protocol::fmp4::{Fmp4Encoder, Track};

#[allow(unused)]
pub(crate) async fn run_server(addr: String) -> anyhow::Result<()> {
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


    let meta_data = meta_data_map()
        .get(stream_name)
        .map(|it| it.value().clone())
        .ok_or_else(|| anyhow::anyhow!(format!("not found meta_data, stream={}", stream_name)))?;

    let video_header = video_header_map()
        .get(stream_name)
        .map(|it| it.value().clone())
        .ok_or_else(|| anyhow::anyhow!(format!("not found meta_data, stream={}", stream_name)))?;

    let mut sps_list = vec![];
    let mut pps_list = vec![];
    let pioneer_nalus = Nalu::from_rtmp_message(&video_header);
    for nalu in pioneer_nalus {
        match nalu.get_nal_unit_type() {
            Nalu::UNIT_TYPE_SPS => sps_list.push(nalu.as_ref().to_vec()),
            Nalu::UNIT_TYPE_PPS => pps_list.push(nalu.as_ref().to_vec()),
            _ => {}
        }
    }

    let rx = eventbus_map()
        .get(stream_name)
        .map(|it| it.register_receiver())
        .ok_or_else(|| anyhow::anyhow!(format!("not found eventbus, stream={}", stream_name)))?;

    let mut fmp4_encoder = Fmp4Encoder::new(Track {
        duration: meta_data.duration as u32,
        timescale: (meta_data.duration * meta_data.frame_rate) as u32,
        width: meta_data.width as _,
        height: meta_data.height as _,
        sps_list,
        pps_list,
        ..Default::default()
    });

    // send video header
    let header = fmp4_encoder.init_segment();
    outgoing.send(Message::binary(header)).await?;

    while let Ok(msg) = rx.recv().await {
        let nalus = Nalu::from_rtmp_message(&msg);
        for nalu in nalus {
            let bytes = fmp4_encoder.wrap_frame(nalu.as_ref(), nalu.is_key_frame);
            outgoing.send(Message::binary(bytes)).await?;
        }
    }
    log::info!("WebSocket disconnected: {}, stream_name={}", addr, stream_name);
    Ok(())
}

