mod packet;
mod extension;

use async_std::task;
use async_std::net::{TcpListener, ToSocketAddrs};
use async_std::prelude::*;
use async_std::{
    io::BufReader,
    net::TcpStream,
};

use futures::{select, FutureExt, AsyncReadExt};
use futures::channel::mpsc;
use futures::sink::SinkExt;
use std::sync::Arc;
use std::collections::hash_map::{Entry, HashMap};
use crate::extension::{TcpStreamExtend, print_hex};
use crate::packet::{Handshake0, Handshake1, Handshake2, calc_amf_byte_len, read_all_amf_value};
use byteorder::{BigEndian, ByteOrder};
use rand::Rng;
use std::time::{SystemTime, Instant};
use chrono::Local;
use amf::Pair;
use std::io::Write;

pub type BoxResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;

fn init_logger() {
    use chrono::Local;
    use std::io::Write;

    let env = env_logger::Env::default()
        .filter_or(env_logger::DEFAULT_FILTER_ENV, "info");
    // 设置日志打印格式
    env_logger::Builder::from_env(env)
        .format(|buf, record| {
            writeln!(
                buf,
                "{} {} [{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                buf.default_styled_level(record.level()),
                record.module_path().unwrap_or("<unnamed>"),
                &record.args()
            )
        })
        .init();
    log::info!("env_logger initialized.");
}

/// 执行一个新协程，并且在错误时打印错误信息
fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
    where F: Future<Output=BoxResult<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            log::error!("error, {}", e)
        }
    })
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
async fn accept_loop(addr: &str) -> BoxResult<()> {
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

async fn connection_loop(mut stream: TcpStream) -> BoxResult<()> {
    let zero_time = Local::now().timestamp_millis();
    /* C0/S0 */
    let c0 = stream.read_one_return().await?;
    log::info!("C0, version={}", c0);
    stream.write(Handshake0::S0_V3.to_bytes().as_ref()).await?;
    log::info!("S0, version={:?}", Handshake0::S0_V3);

    /* C1/S1 */
    let c1_vec = stream.read_exact_return(Handshake1::PACKET_LENGTH).await?;
    let c1 = Handshake1 {
        time: BigEndian::read_u32(&c1_vec[0..4]),
        zero: BigEndian::read_u32(&c1_vec[4..8]),
        random_data: c1_vec[8..Handshake1::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("C1, {:?}", c1);
    let mut s1 = Handshake1 {
        time: (Local::now().timestamp_millis() - zero_time) as u32,
        zero: 0,
        random_data: gen_random_bytes(1528),
    };
    stream.write(s1.to_bytes().as_ref()).await?;
    log::info!("S1, {:?}", s1);

    /* C2/S2 */
    let mut s2 = Handshake2 {
        time: c1.time,
        time2: 0,
        random_echo: c1.random_data,
    };
    stream.write(s2.to_bytes().as_ref()).await?;
    log::info!("S2, {:?}", s2);
    let c2_vec = stream.read_exact_return(Handshake2::PACKET_LENGTH).await?;
    let c2 = Handshake2 {
        time: BigEndian::read_u32(&c2_vec[0..4]),
        time2: BigEndian::read_u32(&c2_vec[4..8]),
        random_echo: c2_vec[8..Handshake1::PACKET_LENGTH as usize].to_vec(),
    };
    log::info!("C2, {:?}", c2);
    assert_eq!(s1.random_data, c2.random_echo);
    log::info!("Handshake done");

    let set_window_size_packet = stream.read_exact_return(16).await?;
    log::info!("C->S, set_window_size_packet:");
    print_hex(set_window_size_packet.as_ref());

    let connect_packet_header = stream.read_exact_return(12).await?;
    let connect_body_length = BigEndian::read_u24(connect_packet_header[4..7].as_ref());
    let connect_body = stream.read_exact_return(connect_body_length).await?;

    /* parse connect body */
    {
        let values = read_all_amf_value(&connect_body);
        for v in values {
            log::info!("C->S, connect_body part: {:?}", v);
        }
    }

    {
        let ack_window_size = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00];
        stream.write(ack_window_size.as_ref()).await?;
        log::info!("S->C, ack_window_size_packet:");
        print_hex(ack_window_size.to_vec().as_ref());
    }
    {
        let set_peer_bandwidth = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x01];
        stream.write(set_peer_bandwidth.as_ref()).await?;
        log::info!("S->C, set_peer_bandwidth:");
        print_hex(set_peer_bandwidth.to_vec().as_ref());
    }

    {
        let set_chunk_size = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0xA0];
        stream.write(set_chunk_size.as_ref()).await?;
        log::info!("S->C, set_chunk_size:");
        print_hex(set_chunk_size.to_vec().as_ref());
    }

    {
        let ack = stream.read_exact_return(12).await?;
        log::info!("C->S, ack:");
        print_hex(ack.as_ref());
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
        stream.write(response_result.as_ref()).await?;
        log::info!("S->C, response_result:");
        print_hex(response_result.as_ref());
    }

    {
        let one = stream.read_one_return().await?;
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let len = match fmt {
            0 => {
                let h = stream.read_exact_return(11).await?;
                log::info!("C->S, releaseStream header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            1 => {
                let h = stream.read_exact_return(7).await?;
                log::info!("C->S, releaseStream header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            _ => unimplemented!()
        };
        let body = stream.read_exact_return(len).await?;
        log::info!("C->S, releaseStream body:");
        print_hex(body.as_ref());
    }

    {
        let one = stream.read_one_return().await?;
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let len = match fmt {
            0 => {
                let h = stream.read_exact_return(11).await?;
                log::info!("C->S, FCPublish header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            1 => {
                let h = stream.read_exact_return(7).await?;
                log::info!("C->S, FCPublish header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            _ => unimplemented!()
        };
        let body = stream.read_exact_return(len).await?;
        log::info!("C->S, FCPublish body:");
        print_hex(body.as_ref());
    }

    let create_stream_body= {
        let one = stream.read_one_return().await?;
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let len = match fmt {
            0 => {
                let h = stream.read_exact_return(11).await?;
                log::info!("C->S, createStream header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            1 => {
                let h = stream.read_exact_return(7).await?;
                log::info!("C->S, createStream header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            _ => unimplemented!()
        };
        let body = stream.read_exact_return(len).await?;
        log::info!("C->S, createStream body:");
        print_hex(body.as_ref());

        read_all_amf_value(&body)
    };

    {
        let mut response_result: Vec<u8> = vec![0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x14, 0x00, 0x00, 0x00, 0x00, ];
        amf::amf0::Value::String("_result".to_string()).write_to(&mut response_result)?;
        create_stream_body[1].write_to(&mut response_result)?;
        amf::amf0::Value::Null.write_to(&mut response_result)?;
        amf::amf0::Value::Number(1.0).write_to(&mut response_result)?;
        response_result[6] = (response_result.len() - 12) as u8;
        stream.write(response_result.as_ref()).await?;
        log::info!("S->C, response_result:");
        print_hex(response_result.as_ref());
    }

    {
        let one = stream.read_one_return().await?;
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let len = match fmt {
            0 => {
                let h = stream.read_exact_return(11).await?;
                log::info!("C->S, publish header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            1 => {
                let h = stream.read_exact_return(7).await?;
                log::info!("C->S, publish header:");
                print_hex(&h);
                BigEndian::read_u24(&h[3..6])
            }
            _ => unimplemented!()
        };
        let body = stream.read_exact_return(len).await?;

        let values = read_all_amf_value(&body);
        for v in values {
            log::info!("C->S, publish part: {:?}", v);
        }
    };

    {
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
        stream.write(response_result.as_ref()).await?;
        log::info!("S->C, Start publishing:");
        print_hex(response_result.as_ref());
    }

    log::info!("C->S, others:");
    let mut i = 0;
    let mut arr: [char; 8] = ['.'; 8];
    loop {
        let byte = stream.read_one_return().await?;
        print!("{:#04X}", byte);
        if byte.is_ascii_graphic() {
            arr[i % 8] = byte as char;
        } else {
            arr[i % 8] = '.';
        }
        print!(" ");
        std::io::stdout().flush()?;
        i += 1;
        if i % 8 == 0 {
            println!("  {}", arr.iter().collect::<String>());
        }
    }

    Ok(())
}

fn main() -> BoxResult<()> {
    init_logger();
    let server = "127.0.0.1:12345";
    task::block_on(accept_loop(server))
}