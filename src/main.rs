#[macro_use]
extern crate num_derive;

use amf::amf0::Value;
use amf::Pair;
use byteorder::{BigEndian, ByteOrder};
use chrono::Local;
use smol::net::{TcpListener, TcpStream};
use smol::prelude::*;

use protocol::rtmp::*;

use crate::util::{bytes_hex_format, print_hex, spawn_and_log_error, gen_random_bytes};
use crate::eventbus::EventBus;
use once_cell::sync::OnceCell;

mod util;
mod protocol;
mod eventbus;

fn nalu_eventbus() -> &'static EventBus<Vec<u8>> {
    static INSTANCE: OnceCell<EventBus<Vec<u8>>> = OnceCell::new();
    INSTANCE.get_or_init(|| EventBus::with_label("NALU"))
}

fn rtmp_msg_eventbus() -> &'static EventBus<RtmpMessage> {
    static INSTANCE: OnceCell<EventBus<RtmpMessage>> = OnceCell::new();
    INSTANCE.get_or_init(|| EventBus::with_label("RTMP_MSG"))
}

/// TCP 连接处理
async fn accept_loop(addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr.clone()).await?;
    log::info!("Server is listening to {}", addr);

    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        log::info!("new connection: {}", stream.peer_addr()?);
        spawn_and_log_error(connection_loop(stream));
    }
    Ok(())
}

async fn connection_loop(stream: TcpStream) -> anyhow::Result<()> {
    let mut ctx = RtmpContext::new(stream);

    handle_rtmp_handshake(&mut ctx).await?;

    loop {
        let message = RtmpMessage::read_from(&mut ctx).await?;
        log::debug!("[peer={}] C->S, [{}] csid={}, msid={}", ctx.peer_addr, message.message_type_desc(), &message.header.csid, &message.header.msid);
        match message.header.message_type {
            ChunkMessageType::SetChunkSize => {
                ctx.chunk_size = BigEndian::read_u32(&message.body);
                log::info!("[peer={}] C->S, [{}] value={}", ctx.peer_addr, message.message_type_desc(), &ctx.chunk_size);
            }
            ChunkMessageType::UserControlMessage => {
                let bytes = &message.body;
                let event_type = BigEndian::read_u16(&bytes[0..2]);
                // set buffer length
                if event_type == 3 {
                    // 等于 create_stream 应答中第4个字段值
                    let stream_id = BigEndian::read_u32(&bytes[2..6]);
                    let buffer_length = BigEndian::read_u32(&bytes[6..10]);
                    log::info!("[peer={}] C->S, [{}] set buffer length={}, streamId={}", ctx.peer_addr, message.message_type_desc(), buffer_length, stream_id);
                    response_play(&mut ctx, stream_id).await?;
                    send_meta_data_for_play(&mut ctx, &RtmpMetaData {
                        width: 1280.0,
                        height: 720.0,
                        video_codec_id: "avc1".to_string(),
                        video_data_rate: 2000.0,
                        audio_codec_id: "mp4a".to_string(),
                        audio_data_rate: 160.0,
                        frame_rate: 30.0,
                        duration: 30.0,
                    }).await?;
                    // TODO send video stream to peer
                    fn is_key_frame(msg: &RtmpMessage) -> bool {
                        if let ChunkMessageType::VideoMessage = msg.header.message_type {
                            msg.body[0] >> 4 == 1
                        } else {
                            false
                        }
                    }

                    let mut wait_key_frame = true;
                    let receiver = rtmp_msg_eventbus().register_receiver();
                    while let Ok(mut msg) = receiver.recv().await {
                        if wait_key_frame && !is_key_frame(&msg) {
                            continue;
                        }
                        wait_key_frame = false;

                        msg.header.msid = 1;
                        msg.header.timestamp = (Local::now().timestamp_millis() - ctx.ctx_begin_timestamp) as u32;
                        let chunks = msg.split_chunks_bytes(ctx.chunk_size);
                        for chunk in chunks {
                            ctx.write_to_peer(&chunk).await?;
                        }
                    }
                } else {
                    log::info!("[peer={}] C->S, [{}] \n{}", ctx.peer_addr, message.message_type_desc(), bytes_hex_format(&message.body));
                }
            }
            ChunkMessageType::AMF0CommandMessage => {
                let option = message.try_read_body_to_amf0();
                if option.is_none() {
                    log::error!("[peer={}] C->S, ctx={:#?} \n msg={:#?}", ctx.peer_addr, &ctx, &message);
                    unreachable!();
                }
                let values = option.unwrap();
                let command = values[0].try_as_str().unwrap();
                for v in &values {
                    log::info!("[peer={}] C->S, {} part: {:?}", ctx.peer_addr, command, v);
                }

                match command {
                    "connect" => {
                        response_connect(&mut ctx).await?;
                    }
                    "createStream" => {
                        response_create_stream(&mut ctx, &values[1]).await?;
                    }
                    "publish" => {
                        response_publish(&mut ctx).await?;
                    }
                    _ => ()
                }
            }
            ChunkMessageType::AMF0DataMessage => {
                let values = message.try_read_body_to_amf0().unwrap();
                let command = values[0].try_as_str().unwrap();
                for v in &values {
                    if let Value::EcmaArray { entries } = v {
                        log::info!("[peer={}] C->S, [{}] part Array: ", ctx.peer_addr, command);
                        for item in entries {
                            log::info!("[peer={}] C->S, [{}] item: {:?}", ctx.peer_addr, command, item);
                        }
                    } else {
                        log::info!("[peer={}] C->S, [{}] part: {:?}", ctx.peer_addr, command, v);
                    }
                }
            }

            ChunkMessageType::VideoMessage => {
                log::debug!("[peer={}] C->S, [{}] header={:?}", ctx.peer_addr, message.message_type_desc(), message.header);
                rtmp_msg_eventbus().publish(message.clone()).await;
                handle_video_data(&message.body, &ctx);
            }
            ChunkMessageType::AudioMessage => {
                // sender.send(message).await?;
            }
            _ => {
                log::info!("[peer={}] C->S, [{}] OTHER len={}", ctx.peer_addr, message.message_type_desc(), message.header.message_length);
            }
        }
    }

// log::info!("C->S, others:");
// let mut i = 0;
// let mut arr: [char; 8] = ['.'; 8];
// loop {
//     let byte = stream.read_one_return().await?;
//     print!("{:#04X}", byte);
//     if byte.is_ascii_graphic() {
//         arr[i % 8] = byte as char;
//     } else {
//         arr[i % 8] = '.';
//     }
//     print!(" ");
//     std::io::stdout().flush()?;
//     i += 1;
//     if i % 8 == 0 {
//         println!("  {}", arr.iter().collect::<String>());
//     }
// }
}

/// 处理RTMP握手流程
async fn handle_rtmp_handshake(ctx: &mut RtmpContext) -> anyhow::Result<()> {
    /* C0/C1 */
    let c0 = ctx.read_exact_from_peer(1).await?[0];
    log::info!("[peer={}] C0, version={}", ctx.peer_addr, c0);

    let c1_vec = ctx.read_exact_from_peer(Handshake1::PACKET_LENGTH).await?;
    let c1 = Handshake1 {
        time: BigEndian::read_u32(&c1_vec[0..4]),
        zero: BigEndian::read_u32(&c1_vec[4..8]),
        random_data: c1_vec[8..Handshake1::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("[peer={}] C1, {:?}", ctx.peer_addr, c1);

    /* S0/S1/S2 */
    ctx.write_to_peer(Handshake0::S0_V3.to_bytes().as_ref()).await?;
    log::info!("[peer={}] S0, version={:?}", ctx.peer_addr, Handshake0::S0_V3);

    let s1 = Handshake1 {
        time: (Local::now().timestamp_millis() - ctx.ctx_begin_timestamp) as u32,
        zero: 0,
        random_data: gen_random_bytes(1528),
    };
    ctx.write_to_peer(s1.to_bytes().as_ref()).await?;
    log::info!("[peer={}] S1, {:?}", ctx.peer_addr, s1);

    let s2 = Handshake2 {
        time: c1.time,
        time2: 0,
        random_echo: c1.random_data,
    };
    ctx.write_to_peer(s2.to_bytes().as_ref()).await?;
    log::info!("[peer={}] S2, {:?}", ctx.peer_addr, s2);


    /* C2*/
    let c2_vec = ctx.read_exact_from_peer(Handshake2::PACKET_LENGTH).await?;
    let c2 = Handshake2 {
        time: BigEndian::read_u32(&c2_vec[0..4]),
        time2: BigEndian::read_u32(&c2_vec[4..8]),
        random_echo: c2_vec[8..Handshake2::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("[peer={}] C2, {:?}", ctx.peer_addr, c2);
    assert_eq!(s1.random_data, c2.random_echo);

    ctx.recv_bytes_num += 1 + Handshake1::PACKET_LENGTH + Handshake2::PACKET_LENGTH;
    Ok(())
}

async fn response_connect(ctx: &mut RtmpContext) -> anyhow::Result<()> {
    {
        let ack_window_size = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00];
        ctx.write_to_peer(ack_window_size.as_ref()).await?;
        log::info!("[peer={}] S->C, ack_window_size_packet:", ctx.peer_addr);
        print_hex(ack_window_size.to_vec().as_ref());
    }

    if ctx.chunk_size == 128 {
        ctx.chunk_size = 4096;
    }

    {
        let mut set_peer_bandwidth = vec![
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x05, 0x06,
            0x00, 0x00, 0x00, 0x00];
        set_peer_bandwidth.append(&mut ctx.chunk_size.to_be_bytes().to_vec());
        // 0-Hard, 1-Soft, 2-Dynamic
        set_peer_bandwidth.push(0x01);

        ctx.write_to_peer(&set_peer_bandwidth).await?;
        log::info!("[peer={}] S->C, set_peer_bandwidth:", ctx.peer_addr);
        print_hex(set_peer_bandwidth.to_vec().as_ref());
    }

    {
        let mut set_chunk_size = vec![
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x01,
            0x00, 0x00, 0x00, 0x00];
        set_chunk_size.append(&mut ctx.chunk_size.to_be_bytes().to_vec());
        ctx.write_to_peer(set_chunk_size.as_ref()).await?;
        log::info!("[peer={}] S->C, set_chunk_size:", ctx.peer_addr);
        print_hex(set_chunk_size.to_vec().as_ref());
    }

    {
        let mut response_result: Vec<u8> = vec![0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x14, 0x00, 0x00, 0x00, 0x00, ];
        amf::amf0::Value::String("_result".to_string()).write_to(&mut response_result)?;
        amf::amf0::Value::Number(1.0).write_to(&mut response_result)?;
        amf::amf0::Value::Object {
            class_name: None,
            entries: vec![
                Pair {
                    key: "fmsVer".to_owned(),
                    value: amf::amf0::Value::String("FMS/3,0,1,123".to_owned()),
                },
                Pair {
                    key: "capabilities".to_owned(),
                    value: amf::amf0::Value::Number(31.0),
                },
            ],
        }.write_to(&mut response_result)?;
        amf::amf0::Value::Object {
            class_name: None,
            entries: vec![
                Pair {
                    key: "level".to_owned(),
                    value: amf::amf0::Value::String("status".to_owned()),
                },
                Pair {
                    key: "code".to_owned(),
                    value: amf::amf0::Value::String("NetConnection.Connect.Success".to_owned()),
                },
                Pair {
                    key: "description".to_owned(),
                    value: amf::amf0::Value::String("Connection succeeded.".to_owned()),
                },
                Pair {
                    key: "objectEncoding".to_owned(),
                    value: amf::amf0::Value::Number(0.0),
                },
            ],
        }.write_to(&mut response_result)?;
        response_result[6] = (response_result.len() - 12) as u8;
        ctx.write_to_peer(response_result.as_ref()).await?;
        log::info!("[peer={}] S->C, response_result:", ctx.peer_addr);
        print_hex(response_result.as_ref());
    }

    Ok(())
}

async fn response_create_stream(ctx: &mut RtmpContext, prev_command_number: &amf::amf0::Value) -> anyhow::Result<()> {
    let mut response_result: Vec<u8> = vec![0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x14, 0x00, 0x00, 0x00, 0x00, ];
    amf::amf0::Value::String("_result".to_string()).write_to(&mut response_result)?;
    prev_command_number.write_to(&mut response_result)?;
    amf::amf0::Value::Null.write_to(&mut response_result)?;
    amf::amf0::Value::Number(9.0).write_to(&mut response_result)?;
    response_result[6] = (response_result.len() - 12) as u8;
    ctx.write_to_peer(response_result.as_ref()).await?;
    log::info!("[peer={}] S->C, response_result:", ctx.peer_addr);
    print_hex(response_result.as_ref());

    Ok(())
}

async fn response_publish(ctx: &mut RtmpContext) -> anyhow::Result<()> {
    let mut response_result: Vec<u8> = vec![0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x14, 0x00, 0x00, 0x00, 0x01, ];
    amf::amf0::Value::String("onStatus".to_string()).write_to(&mut response_result)?;
    amf::amf0::Value::Number(1.0).write_to(&mut response_result)?;
    amf::amf0::Value::Null.write_to(&mut response_result)?;
    amf::amf0::Value::Object {
        class_name: None,
        entries: vec![
            Pair {
                key: "level".to_owned(),
                value: amf::amf0::Value::String("status".to_owned()),
            },
            Pair {
                key: "code".to_owned(),
                value: amf::amf0::Value::String("NetStream.Publish.Start".to_owned()),
            },
            Pair {
                key: "description".to_owned(),
                value: amf::amf0::Value::String("Start publishing".to_owned()),
            }
        ],
    }.write_to(&mut response_result)?;
    response_result[6] = (response_result.len() - 12) as u8;
    ctx.write_to_peer(response_result.as_ref()).await?;
    log::info!("[peer={}] S->C, Start publishing:", ctx.peer_addr);
    print_hex(response_result.as_ref());

    Ok(())
}

async fn response_play(ctx: &mut RtmpContext, stream_id: u32) -> anyhow::Result<()> {
    {
        let mut rs: Vec<u8> = vec![
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x02, 0x04,
            0x00, 0x00, 0x00, 0x01,
            0x00, 0x00,
        ];
        ctx.write_to_peer(rs.as_ref()).await?;
        log::info!("[peer={}] S->C, Stream Begin, streamId={}", ctx.peer_addr, &stream_id);
    }

    {
        let mut response_result: Vec<u8> = vec![
            0x05, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x14,
            0x00, 0x00, 0x00, 0x01,
        ];
        amf::amf0::Value::String("onStatus".to_string()).write_to(&mut response_result)?;
        amf::amf0::Value::Number(0.0).write_to(&mut response_result)?;
        amf::amf0::Value::Null.write_to(&mut response_result)?;
        amf::amf0::Value::Object {
            class_name: None,
            entries: vec![
                Pair {
                    key: "level".to_owned(),
                    value: amf::amf0::Value::String("status".to_owned()),
                },
                Pair {
                    key: "code".to_owned(),
                    value: amf::amf0::Value::String("NetStream.Play.Start".to_owned()),
                },
                Pair {
                    key: "description".to_owned(),
                    value: amf::amf0::Value::String("Start live".to_owned()),
                }
            ],
        }.write_to(&mut response_result)?;
        response_result[6] = (response_result.len() - 12) as u8;
        ctx.write_to_peer(response_result.as_ref()).await?;
        log::info!("[peer={}] S->C, Start play:", ctx.peer_addr);
        print_hex(response_result.as_ref());
    }

    {
        let mut response_result: Vec<u8> = vec![
            0x05, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x14,
            0x00, 0x00, 0x00, 0x01,
        ];
        amf::amf0::Value::String("|RtmpSampleAccess".to_string()).write_to(&mut response_result)?;
        amf::amf0::Value::Boolean(true).write_to(&mut response_result)?;
        amf::amf0::Value::Boolean(true).write_to(&mut response_result)?;
        response_result[6] = (response_result.len() - 12) as u8;
        ctx.write_to_peer(response_result.as_ref()).await?;
        log::info!("[peer={}] S->C, Start play:", ctx.peer_addr);
        print_hex(response_result.as_ref());
    }
    Ok(())
}

/// # 向对端发送onMetaData数据
///
/// 在publish或者play之后就是开始传输媒体数据了，媒体数据分为3种，
/// script脚本数据、video视频数据、audio音频数据。
///
/// 首先需要传输的是脚本数据onMetaData，也称为元数据。
///
/// onMetaData主要描述音视频的编码格式的相关参数。
async fn send_meta_data_for_play(ctx: &mut RtmpContext, meta_data: &RtmpMetaData) -> anyhow::Result<()> {
    let mut response_result: Vec<u8> = vec![
        0x05, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x12,
        0x00, 0x00, 0x00, 0x01,
    ];
    amf::amf0::Value::String("onMetaData".to_string()).write_to(&mut response_result)?;
    amf::amf0::Value::Object {
        class_name: None,
        entries: vec![
            Pair {
                key: "Server".to_owned(),
                value: amf::amf0::Value::String("RIVER".to_owned()),
            },
            Pair {
                key: "width".to_owned(),
                value: amf::amf0::Value::Number(meta_data.width),
            },
            Pair {
                key: "height".to_owned(),
                value: amf::amf0::Value::Number(meta_data.height),
            },
            Pair {
                key: "displayWidth".to_owned(),
                value: amf::amf0::Value::Number(meta_data.width),
            },
            Pair {
                key: "displayHeight".to_owned(),
                value: amf::amf0::Value::Number(meta_data.height),
            },
            Pair {
                key: "duration".to_owned(),
                value: amf::amf0::Value::Number(meta_data.duration),
            },
            Pair {
                key: "framerate".to_owned(),
                value: amf::amf0::Value::Number(meta_data.frame_rate),
            },
            Pair {
                key: "fps".to_owned(),
                value: amf::amf0::Value::Number(meta_data.frame_rate),
            },
            Pair {
                key: "videocodecid".to_owned(),
                value: amf::amf0::Value::String(meta_data.video_codec_id.to_string()),
            },
            Pair {
                key: "videodatarate".to_owned(),
                value: amf::amf0::Value::Number(meta_data.video_data_rate),
            },
            Pair {
                key: "audiocodecid".to_owned(),
                value: amf::amf0::Value::String(meta_data.audio_codec_id.to_string()),
            },
            Pair {
                key: "audiodatarate".to_owned(),
                value: amf::amf0::Value::Number(meta_data.audio_data_rate),
            },
            Pair {
                key: "profile".to_owned(),
                value: amf::amf0::Value::String(Default::default()),
            },
            Pair {
                key: "level".to_owned(),
                value: amf::amf0::Value::String(Default::default()),
            },
        ],
    }.write_to(&mut response_result)?;
    let be_bytes: [u8; 2] = ((response_result.len() - 12) as u16).to_be_bytes();
    response_result[5] = be_bytes[0];
    response_result[6] = be_bytes[1];
    ctx.write_to_peer(response_result.as_ref()).await?;
    log::info!("[peer={}] S->C, Start play:", ctx.peer_addr);
    print_hex(response_result.as_ref());

    Ok(())
}

fn main() -> anyhow::Result<()> {
    util::init_logger();
    let server = "127.0.0.1:1935";
    smol::block_on(accept_loop(server))
}