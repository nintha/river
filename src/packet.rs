use std::fmt::{Debug, Formatter};

use amf::amf0;
use amf::amf0::Value;
use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use chrono::Local;
use num::FromPrimitive;
use smol::io::{AsyncReadExt, AsyncWriteExt};
use smol::net::TcpStream;

use crate::BoxResult;
use crate::extension::{bytes_hex_format, print_hex};

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
    pub version: u8
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
}

impl RtmpContext {
    pub fn new(stream: TcpStream) -> Self {
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
        }
    }

    pub async fn read_exact_return(&mut self, bytes_num: u32) -> anyhow::Result<Vec<u8>> {
        let mut data = vec![0u8; bytes_num as usize];
        AsyncReadExt::read_exact(&mut self.stream, &mut data).await?;
        Ok(data)
    }

    pub async fn write(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.stream.write_all(bytes).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct RtmpMessageHeader {
    /// chunk stream id
    /// 2 (low level), 3 (high level), 4 (control stream), 5 (video) and 6 (audio).
    pub csid: u8,
    pub timestamp: u32,
    pub message_length: u32,
    pub message_type_id: u8,
    pub message_type: ChunkMessageType,
    /// message stream id
    pub msid: u32,
}

impl RtmpMessageHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut rs = vec![self.csid];
        rs.write_u24::<BigEndian>(self.timestamp).unwrap();
        rs.write_u24::<BigEndian>(self.message_length).unwrap();
        rs.write_u8(self.message_type_id).unwrap();
        rs.write_u32::<BigEndian>(self.msid).unwrap();
        rs
    }
}

pub struct RtmpMessage {
    pub header: RtmpMessageHeader,
    pub body: Vec<u8>,
    pub chunk_count: u32,
}

impl RtmpMessage {
    /// 读取完整消息
    pub async fn read_from(ctx: &mut RtmpContext) -> BoxResult<Self> {
        let mut chunk = RtmpMessage::read_chunk_from(ctx).await?;
        while ctx.remain_message_length > 0 {
            let mut remain_chunk = RtmpMessage::read_chunk_from(ctx).await?;
            chunk.body.append(&mut remain_chunk.body);
            chunk.chunk_count += 1;
        }

        Ok(chunk)
    }

    /// 读取一个消息分片
    async fn read_chunk_from(ctx: &mut RtmpContext) -> BoxResult<Self> {
        let one = ctx.read_exact_return(1).await?[0];
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let (timestamp, message_length, message_type_id, message_stream_id) = match fmt {
            0 => {
                let h = ctx.read_exact_return(11).await?;
                print_hex(&h);
                // 时间差值置零
                ctx.last_timestamp_delta = 0;
                ctx.last_timestamp = BigEndian::read_u24(&h[0..3]);
                ctx.last_message_length = BigEndian::read_u24(&h[3..6]);
                ctx.remain_message_length = 0;
                ctx.last_message_type_id = h[6];
                ctx.last_message_stream_id = BigEndian::read_u32(&h[7..11]);
                ctx.recv_bytes_num += 12;

                (ctx.last_timestamp,
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            1 => {
                let h = ctx.read_exact_return(7).await?;
                // bytes_hex_format(&h);
                let timestamp_delta = BigEndian::read_u24(&h[0..3]);
                ctx.last_message_length = BigEndian::read_u24(&h[3..6]);
                ctx.remain_message_length = 0;
                ctx.last_message_type_id = h[6];
                ctx.last_timestamp += timestamp_delta;
                ctx.recv_bytes_num += 8;

                (ctx.last_timestamp,
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            2 => {
                let h = ctx.read_exact_return(3).await?;
                let timestamp_delta = BigEndian::read_u24(&h[0..3]);
                ctx.last_timestamp_delta = timestamp_delta;
                ctx.last_timestamp += timestamp_delta;
                ctx.recv_bytes_num += 4;

                (ctx.last_timestamp,
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            3 => {
                ctx.last_timestamp += ctx.last_timestamp_delta;
                (ctx.last_timestamp,
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            _ => unreachable!()
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
        let message_data = ctx.read_exact_return(read_num).await?;
        ctx.recv_bytes_num += read_num;

        let message_type = FromPrimitive::from_u8(message_type_id)
            .ok_or(anyhow::anyhow!(format!("invalid message type: {}", message_type_id)))?;
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
            1 => "ProtocolControlMessages::SetChunkSize".into(),
            2 => "ProtocolControlMessages::AbortMessage".into(),
            3 => "ProtocolControlMessages::Acknowledgement".into(),
            4 => "ProtocolControlMessages::UserControlMessage".into(),
            5 => "ProtocolControlMessages::WindowAcknowledgementSize".into(),
            6 => "ProtocolControlMessages::SetPeerBandwidth".into(),
            17 => "CommandMessages::AMF3CommandMessage".into(),
            20 => "CommandMessages::AMF0CommandMessage".into(),
            15 => "CommandMessages::AMF3DataMessage".into(),
            18 => "CommandMessages::AMF0DataMessage".into(),
            16 => "CommandMessages::AMF3SharedObjectMessage".into(),
            19 => "CommandMessages::AMF0SharedObjectMessage".into(),
            8 => "CommandMessages::AudioMessage".into(),
            9 => "CommandMessages::VideoMessage".into(),
            22 => "CommandMessages::AggregateMessage".into(),
            _ => "UnknownMessage".into(),
        }
    }

    /// 把body数据解析成amf0格式
    pub fn try_read_body_to_amf0(&self) -> Option<Vec<Value>> {
        match self.header.message_type_id {
            18 | 19 | 20 => read_all_amf_value(&self.body),
            _ => None
        }
    }

    pub fn split_chunks_bytes(&self, chunk_size: u32, msid: u32) -> Vec<Vec<u8>> {
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
        for item in msid.to_be_bytes().iter().rev() {
            (&mut rs[0]).insert(0, item.clone());
        }
        for item in self.header.to_bytes()[0..8].iter().rev() {
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
        write!(f, "ChunkMessage {{\nheader: {:?}\nmessage type: {}\nchunk count={}\nbody:\n{}}}",
               self.header,
               self.message_type_desc(),
               self.chunk_count,
               bytes_hex_format(&self.body))
    }
}

#[derive(Debug, PartialEq, FromPrimitive)]
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

pub fn calc_amf_byte_len(v: &amf0::Value) -> usize {
    match v {
        Value::Number(_) => 9,
        Value::Boolean(_) => 2,
        Value::String(s) => (s.len() + 3),
        Value::Object { class_name, entries } => {
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

pub fn print_video_data(bytes: &[u8]) {
    let frame_type = bytes[0];
    log::info!("video frame type = {:#04X}", frame_type);
    let mut read_index = 1;
    let acv_packet_type = bytes[read_index];
    read_index += 1;

    let data_len = if acv_packet_type == 0 {
        let composition_time_offset = &bytes[read_index..read_index + 3];
        read_index += 3;
        let avc_profile = &bytes[read_index..read_index + 4];
        read_index += 4;
        let length_size_minus_one = &bytes[read_index];
        read_index += 1;
        let num_of_sps = &bytes[read_index] & 0x1F;
        read_index += 1;
        println!("sps num = {}", num_of_sps);
        for _ in 0..num_of_sps as usize {
            let data_len = BigEndian::read_u16(&bytes[read_index..]);
            read_index += 2;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            println!("len={}, sps data:\n{}", data_len, bytes_hex_format(data));
        }
        let num_of_pps = &bytes[read_index] & 0x1F;
        read_index += 1;
        println!("pps num = {}", num_of_pps);
        for _ in 0..num_of_pps as usize {
            let data_len = BigEndian::read_u16(&bytes[read_index..]);
            read_index += 2;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            println!("len={}, pps data:\n{}", data_len, bytes_hex_format(data));
        }
    } else {
        let composition_time_offset = &bytes[read_index..read_index + 3];
        read_index += 3;
        loop {
            if read_index >= bytes.len() {
                break;
            }
            let data_len = BigEndian::read_u32(&bytes[read_index..]);
            read_index += 4;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            // println!("NALU Type: {}, len={}", nalu_type_desc(&data[0]), data_len);
            println!("len={}, nalu data:\n{}", data_len, bytes_hex_format(data));
        }
    };
}

fn nalu_type_desc(nalu_type: &u8) -> String {
    let priority: String = match (nalu_type & 0x60) >> 5 {
        0 => "DISPOSABLE".into(),
        1 => "LOW".into(),
        2 => "HIGH".into(),
        3 => "HIGHEST".into(),
        _ => "UNKNOWN".into(),
    };

    let t: String = match nalu_type & 0x1F {
        1 => "SLICE".into(),
        2 => "DPA".into(),
        3 => "DPB".into(),
        4 => "DPC".into(),
        5 => "IDR".into(),
        6 => "SEI".into(),
        7 => "SPS".into(),
        8 => "PPS".into(),
        9 => "AUD".into(),
        10 => "EOSEQ".into(),
        11 => "EOSTREAM".into(),
        12 => "FILL".into(),
        _ => "UNKNOWN".into(),
    };

    format!("{}::{}", priority, t)
}


