use chrono::format::Pad::Zero;
use std::collections::HashMap;
use byteorder::{BigEndian, ByteOrder};
use amf::amf0;
use amf::amf0::Value;
use async_std::net::TcpStream;
use crate::BoxResult;
use crate::extension::{TcpStreamExtend, bytes_hex_format};
use std::fmt::{Debug, Formatter, Error};


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
    pub const RANDOM_DATA_LENGTH: u32 = 1528;
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

pub enum ChunkMessageHeaderType {
    /// Type 0 chunk headers are 11 bytes long. This type MUST be used at
    /// the start of a chunk stream, and whenever the stream timestamp goes
    /// backward (e.g., because of a backward seek)
    Type0 {
        /// timestamp (3 bytes): For a type-0 chunk, the absolute timestamp of
        /// the message is sent here. If the timestamp is greater than or
        /// equal to 16777215 (hexadecimal 0xFFFFFF), this field MUST be
        /// 16777215, indicating the presence of the Extended Timestamp field
        /// to encode the full 32 bit timestamp. Otherwise, this field SHOULD
        /// be the entire timestamp.
        timestamp: u32,
        // 3 byte
        message_length: u32,
        //
        message_type_id: u8,
        // LE
        message_stream_id: u32,
    },
    /// Type 1 chunk headers are 7 bytes long. The message stream ID is not
    /// included; this chunk takes the same stream ID as the preceding chunk.
    /// Streams with variable-sized messages (for example, many video
    /// formats) SHOULD use this format for the first chunk of each new
    /// message after the first.
    Type1 {
        // 3 byte
        timestamp_delta: u32,
        // 3 byte
        message_length: u32,
        //
        message_type_id: u8,
    },
    /// Type 2 chunk headers are 3 bytes long. Neither the stream ID nor the
    /// message length is included; this chunk has the same stream ID and
    /// message length as the preceding chunk. Streams with constant-sized
    /// messages (for example, some audio and data formats) SHOULD use this
    /// format for the first chunk of each message after the first.
    Type2 {
        timestamp_delta: u32,
    },
    /// Type 3 chunks have no message header. The stream ID, message length
    /// and timestamp delta fields are not present; chunks of this type take
    /// values from the preceding chunk for the same Chunk Stream ID. When a
    /// single message is split into chunks, all chunks of a message except
    /// the first one SHOULD use this type. Refer to Example 2
    /// (Section 5.3.2.2). A stream consisting of messages of exactly the
    /// same size, stream ID and spacing in time SHOULD use this type for all
    /// chunks after a chunk of Type 2. Refer to Example 1
    /// (Section 5.3.2.1). If the delta between the first message and the
    /// second message is same as the timestamp of the first message, then a
    /// chunk of Type 3 could immediately follow the chunk of Type 0 as there
    /// is no need for a chunk of Type 2 to register the delta. If a Type 3
    /// chunk follows a Type 0 chunk, then the timestamp delta for this Type
    /// 3 chunk is the same as the timestamp of the Type 0 chunk.
    Type3,
}


pub struct ChunkContext {
    pub last_timestamp: u32,
    pub last_timestamp_delta: u32,
    pub last_message_length: u32,
    pub last_message_type_id: u8,
    pub last_message_stream_id: u32,
}

#[derive(Debug)]
pub struct ChunkMessageHeader {
    pub chunk_stream_id: u32,
    pub timestamp: u32,
    pub message_length: u32,
    pub message_type_id: u8,
    pub message_stream_id: u32,
}

pub struct ChunkMessage {
    pub header: ChunkMessageHeader,
    pub message_data: Vec<u8>,
}


impl ChunkMessage {
    pub async fn read_from(stream: &mut TcpStream, ctx: &mut ChunkContext) -> BoxResult<Self> {
        let one = stream.read_one_return().await?;
        let fmt = one >> 6;
        let csid = one << 2 >> 2;
        let (timestamp, message_length, message_type_id, message_stream_id) = match fmt {
            0 => {
                let h = stream.read_exact_return(11).await?;
                (BigEndian::read_u24(&h[0..3]),
                 BigEndian::read_u24(&h[3..6]),
                 h[6],
                 BigEndian::read_u32(&h[7..11]))
            }
            1 => {
                let h = stream.read_exact_return(7).await?;
                let timestamp_delta = BigEndian::read_u24(&h[0..3]);
                ctx.last_timestamp = timestamp_delta;
                (ctx.last_timestamp + timestamp_delta,
                 BigEndian::read_u24(&h[3..6]),
                 h[6],
                 ctx.last_message_stream_id)
            }
            2 => {
                let h = stream.read_exact_return(3).await?;
                (ctx.last_timestamp + BigEndian::read_u24(&h[0..3]),
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            3 => {
                (ctx.last_timestamp + ctx.last_timestamp_delta,
                 ctx.last_message_length,
                 ctx.last_message_type_id,
                 ctx.last_message_stream_id)
            }
            _ => unreachable!()
        };
        let message_data = stream.read_exact_return(message_length).await?;

        Ok(ChunkMessage {
            header: ChunkMessageHeader {
                chunk_stream_id: csid as u32,
                message_stream_id,
                message_length,
                timestamp,
                message_type_id,
            },
            message_data,
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
        if self.header.message_type_id == 20 {
            Some(read_all_amf_value(&self.message_data))
        } else {
            None
        }
    }
}

impl Debug for ChunkMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChunkMessage {{\nheader: {:?}\nmessage type: {}\nbody:\n{}}}",
               self.header,
               self.message_type_desc(),
               bytes_hex_format(&self.message_data))
    }
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
        Value::EcmaArray { entries: _ } => unimplemented!(),
        Value::Array { entries: _ } => unimplemented!(),
        Value::Date { unix_time: _ } => unimplemented!(),
        Value::XmlDocument(_) => unimplemented!(),
        Value::AvmPlus(_) => unimplemented!(),
    }
}

pub fn read_all_amf_value(bytes: &[u8]) -> Vec<Value> {
    let mut read_num = 0;
    let mut list = Vec::new();

    loop {
        let v = amf::amf0::Value::read_from(&mut &bytes[read_num..]).unwrap();
        let len = calc_amf_byte_len(&v);
        read_num += len;
        list.push(v);

        if read_num >= bytes.len() {
            break;
        }
    }
    list
}


