use std::fmt::{Debug, Formatter};

use amf::amf0;
use amf::amf0::Value;
use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use chrono::Local;
use num::FromPrimitive;
use smol::io::{AsyncReadExt, AsyncWriteExt};
use smol::net::TcpStream;

use crate::rtmp_server::eventbus_map;
use crate::util::bytes_hex_format;
use std::convert::TryFrom;

#[derive(Clone, Debug)]
pub struct Handshake0 {
    /// Version (8 bits): In C0, this field identifies the RTMP version
    /// requested by the client. In S0, this field identifies the RTMP
    /// version selected by the server. The version defined by this
    /// specification is 3. Values 0-2 are deprecated values used by
    /// earlier proprietary products; 4-31 are reserved for future
    /// implementations; and 32-255 are not allowed (to allow
    /// distinguishing RTMP from text-based protocols, which always start
    /// with a printable character). A server that does not recognize the
    /// client’s requested version SHOULD respond with 3. The client MAY
    /// choose to degrade to version 3, or to abandon the handshake.
    pub version: u8,
}

impl Handshake0 {
    pub const S0_V3: Handshake0 = Handshake0 { version: 3 };
    pub fn to_bytes(&self) -> Vec<u8> {
        vec![self.version.to_owned()]
    }
}

#[derive(Clone, Debug)]
pub struct Handshake1 {
    /// Time (4 bytes): This field contains a timestamp, which SHOULD be
    /// used as the epoch for all future chunks sent from this endpoint.
    /// This may be 0, or some arbitrary value. To synchronize multiple
    /// chunkstreams, the endpoint may wish to send the current value of
    /// the other chunkstream’s timestamp.
    pub time: u32,
    /// Zero (4 bytes): This field MUST be all 0s.
    pub zero: u32,
    /// Random data (1528 bytes): This field can contain any arbitrary
    /// values. Since each endpoint has to distinguish between the
    /// response to the handshake it has initiated and the handshake
    /// initiated by its peer,this data SHOULD send something sufficiently
    /// random. But there is no need for cryptographically-secure
    /// randomness, or even dynamic values.
    pub random_data: Vec<u8>,
}

impl Handshake1 {
    pub const PACKET_LENGTH: u32 = 1536;
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.append(self.time.to_be_bytes().to_vec().as_mut());
        v.append(self.zero.to_be_bytes().to_vec().as_mut());
        v.append(self.random_data.clone().as_mut());
        v
    }
}

#[derive(Clone, Debug)]
pub struct Handshake2 {
    /// Time (4 bytes): This field MUST contain the timestamp sent by the
    /// peer in S1 (for C2) or C1 (for S2).
    pub time: u32,
    /// Time2 (4 bytes): This field MUST contain the timestamp at which the
    /// previous packet(s1 or c1) sent by the peer was read.
    pub time2: u32,
    /// Random echo (1528 bytes): This field MUST contain the random data
    /// field sent by the peer in S1 (for C2) or S2 (for C1). Either peer
    /// can use the time and time2 fields together with the current
    /// timestamp as a quick estimate of the bandwidth and/or latency of
    /// the connection, but this is unlikely to be useful.
    pub random_echo: Vec<u8>,
}

impl Handshake2 {
    pub const PACKET_LENGTH: u32 = 1536;
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.append(self.time.to_be_bytes().to_vec().as_mut());
        v.append(self.time2.to_be_bytes().to_vec().as_mut());
        v.append(self.random_echo.clone().as_mut());
        v
    }
}

#[derive(Debug)]
pub struct RtmpContext {
    pub stream: TcpStream,
    pub ctx_begin_timestamp: i64,
    pub last_timestamp: u32,
    pub last_timestamp_delta: u32,
    pub last_message_length: u32,
    pub last_message_type_id: u8,
    pub last_message_stream_id: u32,
    pub chunk_size: u32,
    pub remain_message_length: u32,
    pub recv_bytes_num: u32,
    pub peer_addr: String,
    pub stream_name: String,
    pub is_publisher: bool,
}

impl RtmpContext {
    pub fn new(stream: TcpStream) -> Self {
        let peer_addr = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();
        RtmpContext {
            stream,
            ctx_begin_timestamp: Local::now().timestamp_millis(),
            last_timestamp_delta: 0,
            last_timestamp: 0,
            last_message_length: 0,
            last_message_type_id: 0,
            last_message_stream_id: 0,
            chunk_size: 128,
            remain_message_length: 0,
            recv_bytes_num: 0,
            peer_addr,
            stream_name: Default::default(),
            is_publisher: false,
        }
    }

    pub async fn read_exact_from_peer(&mut self, bytes_num: u32) -> anyhow::Result<Vec<u8>> {
        let mut data = vec![0u8; bytes_num as usize];
        AsyncReadExt::read_exact(&mut self.stream, &mut data).await?;
        Ok(data)
    }

    pub async fn write_to_peer(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }
}

impl Drop for RtmpContext {
    fn drop(&mut self) {
        if self.is_publisher {
            eventbus_map().remove(&self.stream_name);
            log::warn!(
                "[{}][RtmpContext] remove eventbus, stream_name={}",
                self.peer_addr,
                self.stream_name
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct RtmpMessageHeader {
    /// chunk stream id
    pub csid: u8,
    pub timestamp: u32,
    pub message_length: u32,
    pub message_type_id: u8,
    pub message_type: ChunkMessageType,
    /// message stream id
    /// 0 => 信令,
    /// 1 => play 信令| publish 信令 | 音视频数据
    pub msid: u32,
}

impl RtmpMessageHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let enable_extend_timestamp_field = self.timestamp >= 0xFFFFFF;

        let mut rs = vec![self.csid];
        if enable_extend_timestamp_field {
            rs.write_u24::<BigEndian>(0xFFFFFF).unwrap();
        } else {
            rs.write_u24::<BigEndian>(self.timestamp).unwrap();
        }
        rs.write_u24::<BigEndian>(self.message_length).unwrap();
        rs.write_u8(self.message_type_id).unwrap();
        rs.write_u32::<BigEndian>(self.msid).unwrap();
        if enable_extend_timestamp_field {
            rs.write_u32::<BigEndian>(self.timestamp).unwrap();
        }
        rs
    }
}

#[derive(Clone)]
pub struct RtmpMessage {
    pub header: RtmpMessageHeader,
    pub body: Vec<u8>,
    pub chunk_count: u32,
}

impl RtmpMessage {
    /// 读取完整消息
    pub async fn read_from(ctx: &mut RtmpContext) -> anyhow::Result<Self> {
        let mut chunk = RtmpMessage::read_chunk_from(ctx).await?;
        while ctx.remain_message_length > 0 {
            let mut remain_chunk = RtmpMessage::read_chunk_from(ctx).await?;
            chunk.body.append(&mut remain_chunk.body);
            chunk.chunk_count += 1;
        }

        Ok(chunk)
    }

    /// 读取一个消息分片
    async fn read_chunk_from(ctx: &mut RtmpContext) -> anyhow::Result<Self> {
        let one = ctx.read_exact_from_peer(1).await?[0];
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let (timestamp, message_length, message_type_id, message_stream_id) = match fmt {
            0 => {
                let h = ctx.read_exact_from_peer(11).await?;
                // print_hex(&h);
                // 时间差值置零
                ctx.last_timestamp_delta = 0;
                ctx.last_timestamp = BigEndian::read_u24(&h[0..3]);

                ctx.last_message_length = BigEndian::read_u24(&h[3..6]);
                ctx.remain_message_length = 0;
                ctx.last_message_type_id = h[6];
                ctx.last_message_stream_id = BigEndian::read_u32(&h[7..11]);
                ctx.recv_bytes_num += 12;
                if ctx.last_timestamp >= 0xFFFFFF {
                    let extend = ctx.read_exact_from_peer(4).await?;
                    ctx.last_timestamp = BigEndian::read_u32(&extend[0..4]);
                    ctx.recv_bytes_num += 4;
                }

                (
                    ctx.last_timestamp,
                    ctx.last_message_length,
                    ctx.last_message_type_id,
                    ctx.last_message_stream_id,
                )
            }
            1 => {
                let h = ctx.read_exact_from_peer(7).await?;
                // bytes_hex_format(&h);
                let timestamp_delta = BigEndian::read_u24(&h[0..3]);
                ctx.last_message_length = BigEndian::read_u24(&h[3..6]);
                ctx.remain_message_length = 0;
                ctx.last_message_type_id = h[6];
                ctx.last_timestamp += timestamp_delta;
                ctx.recv_bytes_num += 8;

                (
                    ctx.last_timestamp,
                    ctx.last_message_length,
                    ctx.last_message_type_id,
                    ctx.last_message_stream_id,
                )
            }
            2 => {
                let h = ctx.read_exact_from_peer(3).await?;
                let timestamp_delta = BigEndian::read_u24(&h[0..3]);
                ctx.last_timestamp_delta = timestamp_delta;
                ctx.last_timestamp += timestamp_delta;
                ctx.recv_bytes_num += 4;

                (
                    ctx.last_timestamp,
                    ctx.last_message_length,
                    ctx.last_message_type_id,
                    ctx.last_message_stream_id,
                )
            }
            3 => {
                ctx.last_timestamp += ctx.last_timestamp_delta;
                (
                    ctx.last_timestamp,
                    ctx.last_message_length,
                    ctx.last_message_type_id,
                    ctx.last_message_stream_id,
                )
            }
            _ => unreachable!(),
        };

        // 当前分片的body长度
        let read_num = {
            let remain_length = if ctx.remain_message_length > 0 {
                ctx.remain_message_length
            } else {
                message_length
            };

            if remain_length > ctx.chunk_size {
                ctx.remain_message_length = remain_length - ctx.chunk_size;
                ctx.chunk_size
            } else {
                ctx.remain_message_length = 0;
                remain_length
            }
        };
        let message_data = ctx.read_exact_from_peer(read_num).await?;
        ctx.recv_bytes_num += read_num;

        let message_type = FromPrimitive::from_u8(message_type_id).ok_or(anyhow::anyhow!(
            format!("invalid message type: {}", message_type_id)
        ))?;
        Ok(RtmpMessage {
            header: RtmpMessageHeader {
                csid,
                msid: message_stream_id,
                message_length,
                timestamp,
                message_type_id,
                message_type,
            },
            body: message_data,
            chunk_count: 1,
        })
    }

    pub fn message_type_desc(&self) -> String {
        match self.header.message_type_id {
            1 => "ProtocolControlMessages::SetChunkSize",
            2 => "ProtocolControlMessages::AbortMessage",
            3 => "ProtocolControlMessages::Acknowledgement",
            4 => "ProtocolControlMessages::UserControlMessage",
            5 => "ProtocolControlMessages::WindowAcknowledgementSize",
            6 => "ProtocolControlMessages::SetPeerBandwidth",
            17 => "CommandMessages::AMF3CommandMessage",
            20 => "CommandMessages::AMF0CommandMessage",
            15 => "CommandMessages::AMF3DataMessage",
            18 => "CommandMessages::AMF0DataMessage",
            16 => "CommandMessages::AMF3SharedObjectMessage",
            19 => "CommandMessages::AMF0SharedObjectMessage",
            8 => "CommandMessages::AudioMessage",
            9 => "CommandMessages::VideoMessage",
            22 => "CommandMessages::AggregateMessage",
            _ => "UnknownMessage",
        }
            .to_string()
    }

    /// 把body数据解析成amf0格式
    pub fn try_read_body_to_amf0(&self) -> Option<Vec<Value>> {
        match self.header.message_type_id {
            18 | 19 | 20 => read_all_amf_value(&self.body),
            _ => None,
        }
    }

    /// 把一个长message分离成多个chunk，第一个chunk的type=0，后续的type=3
    pub fn split_chunks_bytes(&self, chunk_size: u32) -> Vec<Vec<u8>> {
        let chunk_size = chunk_size as usize;
        let mut rs = vec![];

        let mut remain = self.body.clone();
        while remain.len() > chunk_size {
            let right = remain.split_off(chunk_size);
            rs.push(remain);
            remain = right;
        }
        rs.push(remain);

        // 添加type0头部
        for item in self.header.to_bytes().iter().rev() {
            (&mut rs[0]).insert(0, item.clone());
        }

        // 添加type3头部
        if rs.len() > 1 {
            let type3_fmt = 0xC0 | self.header.csid;
            for item in &mut rs[1..] {
                item.insert(0, type3_fmt);
            }
        }

        rs
    }
}

impl Debug for RtmpMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ChunkMessage {{\nheader: {:?}\nmessage type: {}\nchunk count={}\nbody:\n{}}}",
            self.header,
            self.message_type_desc(),
            self.chunk_count,
            bytes_hex_format(&self.body)
        )
    }
}

#[derive(Debug, PartialEq, FromPrimitive, Clone, Copy)]
pub enum ChunkMessageType {
    SetChunkSize = 1,
    AbortMessage = 2,
    Acknowledgement = 3,
    UserControlMessage = 4,
    WindowAcknowledgementSize = 5,
    SetPeerBandwidth = 6,
    AMF3CommandMessage = 17,
    AMF0CommandMessage = 20,
    AMF3DataMessage = 15,
    AMF0DataMessage = 18,
    AMF3SharedObjectMessage = 16,
    AMF0SharedObjectMessage = 19,
    AudioMessage = 8,
    VideoMessage = 9,
    AggregateMessage = 22,
}

#[derive(Default)]
pub struct RtmpMetaData {
    pub width: f64,
    pub height: f64,
    pub video_codec_id: String,
    pub video_data_rate: f64,
    pub audio_codec_id: String,
    pub audio_data_rate: f64,
    pub frame_rate: f64,
    pub duration: f64,
    pub begin_time: i64,
}

impl TryFrom<&amf::amf0::Value> for RtmpMetaData {
    type Error = anyhow::Error;

    fn try_from(value: &amf::amf0::Value) -> Result<Self, Self::Error> {
        let mut meta_data = RtmpMetaData::default();
        if let Value::EcmaArray { entries } = value {
            for item in entries {
                match item.key.as_ref() {
                    "duration" => {
                        meta_data.duration = item.value.try_as_f64().unwrap_or_default();
                    }
                    "width" => {
                        meta_data.width = item.value.try_as_f64().unwrap_or_default();
                    }
                    "height" => {
                        meta_data.height = item.value.try_as_f64().unwrap_or_default();
                    }
                    "videocodecid" => {
                        meta_data.video_codec_id =
                            item.value.try_as_str().unwrap_or_default().to_owned();
                    }
                    "videodatarate" => {
                        meta_data.video_data_rate =
                            item.value.try_as_f64().unwrap_or_default();
                    }
                    "framerate" => {
                        meta_data.frame_rate =
                            item.value.try_as_f64().unwrap_or_default();
                    }
                    "audiocodecid" => {
                        meta_data.audio_codec_id =
                            item.value.try_as_str().unwrap_or_default().to_owned();
                    }
                    "audiodatarate" => {
                        meta_data.audio_data_rate =
                            item.value.try_as_f64().unwrap_or_default();
                    }
                    _ => {}
                }
            }
            meta_data.begin_time = Local::now().timestamp_millis();
            Ok(meta_data)
        } else {
            Err(anyhow::anyhow!("value is not Value::EcmaArray"))?
        }
    }
}

pub fn calc_amf_byte_len(v: &amf0::Value) -> usize {
    match v {
        Value::Number(_) => 9,
        Value::Boolean(_) => 2,
        Value::String(s) => (s.len() + 3),
        Value::Object { entries, .. } => {
            // marker and tail
            let mut len = 4;
            for en in entries {
                len += en.key.len() + 2;
                len += calc_amf_byte_len(&en.value);
            }
            len
        }
        Value::Null => 1,
        Value::Undefined => 1,
        Value::EcmaArray { entries } => {
            // marker and tail
            let mut len = 8;
            for en in entries {
                len += en.key.len() + 2;
                len += calc_amf_byte_len(&en.value);
            }
            len
        }
        Value::Array { entries: _ } => unimplemented!(),
        Value::Date { unix_time: _ } => unimplemented!(),
        Value::XmlDocument(_) => unimplemented!(),
        Value::AvmPlus(_) => unimplemented!(),
    }
}

pub fn read_all_amf_value(bytes: &[u8]) -> Option<Vec<Value>> {
    let mut read_num = 0;
    let mut list = Vec::new();

    loop {
        if let Ok(v) = amf::amf0::Value::read_from(&mut &bytes[read_num..]) {
            let len = calc_amf_byte_len(&v);
            read_num += len;
            list.push(v);

            if read_num >= bytes.len() {
                break;
            }
        } else {
            return None;
        }
    }
    Some(list)
}
