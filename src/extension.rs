use async_std::{
    io::BufReader,
    net::TcpStream,
};
use async_trait::async_trait;
use futures::AsyncReadExt;
use super::BoxResult;
use std::error::Error;

#[async_trait]
pub trait TcpStreamExtend {
    /// blocking read some bytes from `TcpStream`
    async fn read_exact_return(&mut self, bytes_num: u32) -> BoxResult<Vec<u8>>;
    /// blocking read one byte from `TcpStream`
    async fn read_one_return(&mut self) -> BoxResult<u8> {
        self.read_exact_return(1).await.map(|v| v[0])
    }
}

#[async_trait]
impl TcpStreamExtend for TcpStream {
    /// blocking read some bytes from `TcpStream`
    async fn read_exact_return(&mut self, bytes_num: u32) -> BoxResult<Vec<u8>> {
        let mut data = vec![0u8; bytes_num as usize];
        self.read_exact(&mut data).await?;
        return Ok(data);
    }
}

pub fn bytes_hex_format(bytes: &[u8]) -> String {
    let mut text = String::new();
    let mut i = 0;
    let mut arr: [char; 8] = ['.'; 8];
    for byte in bytes {
        text += &format!("{:#04X}", byte);
        if byte.is_ascii_graphic() {
            arr[i % 8] = byte.clone() as char;
        } else {
            arr[i % 8] = '.';
        }
        text += &format!(" ");
        i += 1;
        if i % 8 == 0 {
            text += &format!("  {}\n", arr.iter().collect::<String>());
        }
    }
    if i % 8 > 0 {
        for _ in 0..(7 - (i - 1) % 8) {
            text += "     ";
        }
        text += &format!("  {}\n", arr.iter().take(((i - 1) % 8) + 1).collect::<String>());
    }
    text
}

pub fn print_hex(bytes: &[u8]) {
    println!("{}", bytes_hex_format(bytes));
}