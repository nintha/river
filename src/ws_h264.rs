use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use async_tungstenite::tungstenite::Message;
use crossbeam_utils::atomic::AtomicCell;
use futures::sink::SinkExt;
use futures::StreamExt;
use smol::net::{SocketAddr, TcpListener, TcpStream};

use crate::protocol::h264::Nalu;
use crate::rtmp_server::{eventbus_map, video_header_map, audio_header_map};
use crate::protocol::rtmp::{ChunkMessageType, RtmpMessage};
use smol::channel::Receiver;
use smol::stream::{Stream};
use smol::stream;
use crate::protocol::aac::{AAC, ADTS};

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

    // send video header
    if let Some(header) = video_header_map().get(stream_name) {
        for mix in Mix::from_rtmp_message(&header, &stream_name) {
            outgoing.send(Message::binary(mix.to_bytes())).await?;
        }
    }

    if let Some(el) = eventbus_map().get(stream_name) {
        let rx = el.register_receiver();
        std::mem::drop(el);

        let rx = rtmp_rx_into_mix_rx(rx, stream_name.to_string());
        futures::pin_mut!(rx);

        while let Some(mix) = StreamExt::next(&mut rx).await {
            outgoing.send(Message::binary(mix.to_bytes())).await?;
        }
    }
    log::info!("WebSocket disconnected: {}, stream_name={}", addr, stream_name);
    Ok(())
}

// 把RMTP流转换城MIX流，并保证首帧为关键帧
fn rtmp_rx_into_mix_rx(rx: Receiver<RtmpMessage>, stream_name: String) -> impl Stream<Item=Mix> {
    stream::unfold((rx, false, stream_name), |(rx, first_key_frame, stream_name)| async move {
        while let Ok(msg) = rx.recv().await {
            let mixes = Mix::from_rtmp_message(&msg, &stream_name);
            if mixes.is_empty() {
                continue;
            }

            if first_key_frame {
                return Some((stream::iter(mixes), (rx, first_key_frame, stream_name)));
            }

            let mut mixes = mixes.into_iter().skip_while(|mix| !mix.is_key_frame()).collect::<Vec<Mix>>();

            // 消息堆积，丢弃非关键帧
            if rx.len() > 3 {
                mixes.retain(|x| x.is_key_frame());
            }

            if mixes.is_empty() {
                continue;
            }

            return Some((stream::iter(mixes), (rx, true, stream_name)));
        }
        None
    }).flatten()
}

/// 媒体混合数据
enum Mix {
    Video(Nalu),
    Audio(ADTS),
}

impl Mix {
    const VIDEO_FLAG: u8 = 0x00;
    const AUDIO_FLAG: u8 = 0x01;
    pub fn from_rtmp_message(msg: &RtmpMessage, stream_name: &str) -> Vec<Self> {
        match msg.header.message_type {
            ChunkMessageType::VideoMessage => {
                Nalu::from_rtmp_message(&msg).into_iter().map(Mix::Video).collect()
            }
            ChunkMessageType::AudioMessage => {
                if let Some(header) = audio_header_map().get(stream_name) {
                    AAC::from_rtmp_message(&msg, header.value())
                        .into_iter()
                        .map(|x| x.to_adts())
                        .flatten()
                        .map(Mix::Audio)
                        .collect()
                } else {
                    vec![]
                }
            }
            _ => vec![]
        }
    }
    #[allow(unused)]
    pub fn is_video(&self) -> bool {
        matches!(self, Mix::Video(_))
    }
    #[allow(unused)]
    pub fn is_audio(&self) -> bool {
        matches!(self, Mix::Audio(_))
    }

    pub fn is_key_frame(&self) -> bool {
        if let Mix::Video(nalu) = self {
            nalu.is_key_frame
        } else {
            false
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        match self {
            Mix::Video(nalu) => {
                let mut bytes = vec![Mix::VIDEO_FLAG];
                bytes.extend_from_slice(nalu.as_ref());
                bytes
            }
            Mix::Audio(aac) => {
                let mut bytes = vec![Mix::AUDIO_FLAG];
                bytes.extend_from_slice(&aac.to_bytes());
                bytes
            }
        }
    }
}

