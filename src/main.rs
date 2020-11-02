#[macro_use]
extern crate num_derive;
use amf::amf0::Value;
use amf::Pair;
use byteorder::{BigEndian, ByteOrder};
use chrono::Local;
use rand::Rng;
use smol::net::{TcpListener, TcpStream};
use smol::prelude::*;

use protocol::rtmp::*;

use crate::util::{bytes_hex_format, print_hex};

mod util;
mod protocol;

/// 执行一个新协程，并且在错误时打印错误信息
fn spawn_and_log_error<F>(fut: F) where F: Future<Output=anyhow::Result<()>> + Send + 'static {
    smol::spawn(async move {
        if let Err(e) = fut.await {
            log::error!("spawn future error, {:?}", e)
        }
    }).detach();
}

fn gen_random_bytes(len: u32) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut vec = Vec::new();
    for _ in 0..len {
        vec.push(rng.gen());
    }
    vec
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
        log::debug!("C->S, [{}] csid={}, msid={}", message.message_type_desc(), &message.header.csid, &message.header.msid);
        match message.header.message_type {
            ChunkMessageType::SetChunkSize => {
                ctx.chunk_size = BigEndian::read_u32(&message.body);
                log::info!("C->S, [{}] value={}", message.message_type_desc(), &ctx.chunk_size);
            }
            ChunkMessageType::UserControlMessage => {
                let bytes = &message.body;
                let event_type = BigEndian::read_u16(&bytes[0..2]);
                // set buffer length
                if event_type == 3 {
                    // 等于 create_stream 应答中第4个字段值
                    let stream_id = BigEndian::read_u32(&bytes[2..6]);
                    let buffer_length = BigEndian::read_u32(&bytes[6..10]);
                    log::info!("C->S, [{}] set buffer length={}, streamId={}", message.message_type_desc(), buffer_length, stream_id);
                    response_play(&mut ctx, stream_id).await?;
                    // TODO send video stream to peer
                } else {
                    log::info!("C->S, [{}] \n{}", message.message_type_desc(), bytes_hex_format(&message.body));
                }
            }
            ChunkMessageType::AMF0CommandMessage => {
                let option = message.try_read_body_to_amf0();
                if option.is_none() {
                    log::error!("C->S, ctx={:?} \n msg={:?}", &ctx, &message);
                    unreachable!();
                }
                let values = option.unwrap();
                let command = values[0].try_as_str().unwrap();
                for v in &values {
                    log::info!("C->S, {} part: {:?}", command, v);
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
                        log::info!("C->S, [{}] part Array: ", command);
                        for item in entries {
                            log::info!("C->S, [{}] item: {:?}", command, item);
                        }
                    } else {
                        log::info!("C->S, [{}] part: {:?}", command, v);
                    }
                }
                // sender.send(message).await?;
            }

            ChunkMessageType::VideoMessage => {
                log::info!("C->S, [{}] header={:?}", message.message_type_desc(), message.header);
                print_video_data(&mut ctx, &message.body);
                // sender.send(message).await?;
            }
            ChunkMessageType::AudioMessage => {
                // sender.send(message).await?;
            }
            _ => {
                log::info!("C->S, [{}] OTHER len={}", message.message_type_desc(), message.header.message_length);
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
    let c0 = ctx.read_exact_from_stream(1).await?[0];
    log::info!("C0, version={}", c0);

    let c1_vec = ctx.read_exact_from_stream(Handshake1::PACKET_LENGTH).await?;
    let c1 = Handshake1 {
        time: BigEndian::read_u32(&c1_vec[0..4]),
        zero: BigEndian::read_u32(&c1_vec[4..8]),
        random_data: c1_vec[8..Handshake1::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("C1, {:?}", c1);

    /* S0/S1/S2 */
    ctx.write_to_stream(Handshake0::S0_V3.to_bytes().as_ref()).await?;
    log::info!("S0, version={:?}", Handshake0::S0_V3);

    let s1 = Handshake1 {
        time: (Local::now().timestamp_millis() - ctx.ctx_begin_timestamp) as u32,
        zero: 0,
        random_data: gen_random_bytes(1528),
    };
    ctx.write_to_stream(s1.to_bytes().as_ref()).await?;
    log::info!("S1, {:?}", s1);

    let s2 = Handshake2 {
        time: c1.time,
        time2: 0,
        random_echo: c1.random_data,
    };
    ctx.write_to_stream(s2.to_bytes().as_ref()).await?;
    log::info!("S2, {:?}", s2);


    /* C2*/
    let c2_vec = ctx.read_exact_from_stream(Handshake2::PACKET_LENGTH).await?;
    let c2 = Handshake2 {
        time: BigEndian::read_u32(&c2_vec[0..4]),
        time2: BigEndian::read_u32(&c2_vec[4..8]),
        random_echo: c2_vec[8..Handshake2::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("C2, {:?}", c2);
    assert_eq!(s1.random_data, c2.random_echo);
    log::info!("Handshake done");

    ctx.recv_bytes_num += 1 + Handshake1::PACKET_LENGTH + Handshake2::PACKET_LENGTH;
    Ok(())
}

async fn response_connect(ctx: &mut RtmpContext) -> anyhow::Result<()> {
    {
        let ack_window_size = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00];
        ctx.write_to_stream(ack_window_size.as_ref()).await?;
        log::info!("S->C, ack_window_size_packet:");
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

        ctx.write_to_stream(&set_peer_bandwidth).await?;
        log::info!("S->C, set_peer_bandwidth:");
        print_hex(set_peer_bandwidth.to_vec().as_ref());
    }

    {
        let mut set_chunk_size = vec![
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x01,
            0x00, 0x00, 0x00, 0x00];
        set_chunk_size.append(&mut ctx.chunk_size.to_be_bytes().to_vec());
        ctx.write_to_stream(set_chunk_size.as_ref()).await?;
        log::info!("S->C, set_chunk_size:");
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
        ctx.write_to_stream(response_result.as_ref()).await?;
        log::info!("S->C, response_result:");
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
    ctx.write_to_stream(response_result.as_ref()).await?;
    log::info!("S->C, response_result:");
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
    ctx.write_to_stream(response_result.as_ref()).await?;
    log::info!("S->C, Start publishing:");
    print_hex(response_result.as_ref());

    Ok(())
}

async fn response_play(ctx: &mut RtmpContext, stream_id: u32) -> anyhow::Result<()> {
    {
        let mut rs: Vec<u8> = vec![
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x06, 0x04,
            0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x01,
            0x00, 0x00,
        ];
        let x: [u8; 4] = stream_id.to_be_bytes();
        rs.append(&mut x.to_vec());
        ctx.write_to_stream(rs.as_ref()).await?;
        log::info!("S->C, Stream Begin, streamId={}", &stream_id);
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
        ctx.write_to_stream(response_result.as_ref()).await?;
        log::info!("S->C, Start play:");
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
        ctx.write_to_stream(response_result.as_ref()).await?;
        log::info!("S->C, Start play:");
        print_hex(response_result.as_ref());
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    util::init_logger();
    let server = "127.0.0.1:1935";
    smol::block_on(accept_loop(server))
}