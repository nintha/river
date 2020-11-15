use crate::util::spawn_and_log_error;
use smol::io::{AsyncReadExt, AsyncWriteExt};
use smol::net::{TcpListener, TcpStream};
use smol::stream::StreamExt;
use crate::rtmp_server::{eventbus_map, video_header_map, audio_header_map, meta_data_map};
use crate::protocol::flv::{FLV_HEADER_WITH_TAG0, FLV_HEADER_ONLY_VIDEO_WITH_TAG0};
use crate::protocol::flv::FlvTag;
use chrono::Local;
use std::convert::TryFrom;
use crate::protocol::rtmp::ChunkMessageType;

pub async fn run_server(addr: &str) -> anyhow::Result<()> {
    // Open up a TCP connection and create a URL.
    let listener = TcpListener::bind(addr).await?;
    let addr = format!("http://{}", listener.local_addr()?);
    log::info!("HTTP-FLV Server is listening to {}", addr);

    // For each incoming TCP connection, spawn a task and call `accept`.
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        spawn_and_log_error(accept(stream));
    }
    Ok(())
}

// Take a TCP stream, and convert it into sequential HTTP request / response pairs.
async fn accept(mut stream: TcpStream) -> anyhow::Result<()> {
    log::info!("[HTTP] new connection from {}", stream.peer_addr()?);
    let mut buffer = [0; 1024];
    stream.read(&mut buffer).await?;
    let req = String::from_utf8_lossy(&buffer[..]);
    let stream_name = get_path(req.as_ref()).map(|x| x.trim_start_matches("/")).unwrap_or_default();
    if let Some(eventbus) = eventbus_map().get(stream_name) {
        let receiver = eventbus.register_receiver();

        let header = "HTTP/1.1 200 OK\r\n\
        Server: river\r\n\
        Content-Type: video/x-flv\r\n\
        Connection: close\r\n\
        Transfer-Encoding: chunked\r\n\
        Cache-Control: no-cache\r\n\
        Access-Control-Allow-Origin: *\r\n\
        \r\n\
        ";
        stream.write_all(header.as_bytes()).await?;
        stream.flush().await?;

        write_chunk(&mut stream, &FLV_HEADER_ONLY_VIDEO_WITH_TAG0).await?;

        // 发送sps/pps帧
        if let Some(msg) = video_header_map().get(stream_name) {
            let flv_tag = FlvTag::try_from(msg.value().clone())?;
            write_chunk(&mut stream, flv_tag.as_ref()).await?;
            write_chunk(&mut stream, &(flv_tag.as_ref().len() as u32).to_be_bytes()).await?;
        };
        // 发送audio帧
        // if let Some(msg) = audio_header_map().get(stream_name) {
        //     let flv_tag = FlvTag::try_from(msg.value().clone())?;
        //     write_chunk(&mut stream, flv_tag.as_ref()).await?;
        //     write_chunk(&mut stream, &(flv_tag.as_ref().len() as u32).to_be_bytes()).await?;
        // };

        let ctx_begin_timestamp = Local::now().timestamp_millis();
        while let Ok(mut msg) = receiver.recv().await {
            if ChunkMessageType::VideoMessage == msg.header.message_type {
                msg.header.timestamp = (Local::now().timestamp_millis() - ctx_begin_timestamp) as u32;
                let flv_tag = FlvTag::try_from(msg)?;
                write_chunk(&mut stream, flv_tag.as_ref()).await?;
                write_chunk(&mut stream, &(flv_tag.as_ref().len() as u32).to_be_bytes()).await?;
                if receiver.len() > 2 {
                    log::warn!("receiver.len={}, stream_name={}", receiver.len(), stream_name);
                }
            }
        }
        write_chunk(&mut stream, b"").await?;
    } else {
        let header = "HTTP/1.1 404 Not Found\r\n\r\n";
        stream.write_all(header.as_bytes()).await?;
        stream.flush().await?;
    }
    Ok(())
}

fn get_path(req: &str) -> Option<&str> {
    let first_line = req.lines().next().unwrap_or_default();
    if first_line.starts_with("GET") {
        return first_line.split_whitespace().skip(1).next();
    }
    None
}

async fn write_chunk(stream: &mut TcpStream, bytes: &[u8]) -> anyhow::Result<()> {
    stream.write_all(format!("{:X}\r\n", bytes.len()).as_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    Ok(())
}