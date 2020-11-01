use async_trait::async_trait;
use super::BoxResult;
use smol::net::TcpStream;
use smol::io::AsyncReadExt;

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
    const COLUMN: usize = 16;
    const COL_SPACE: &str = "  ";
    let mut text = String::new();
    let mut i = 0;
    let mut arr: [char; COLUMN] = ['.'; COLUMN];
    for byte in bytes {
        text += &format!("{:02X}", byte);
        if byte.is_ascii_graphic() {
            arr[i % COLUMN] = byte.clone() as char;
        } else {
            arr[i % COLUMN] = '.';
        }
        text += &format!(" ");
        i += 1;
        // 每8列多一个空格
        if i % 8 == 0 {
            text += COL_SPACE;
        }
        if i % COLUMN == 0 {
            let mut ascii = arr.iter().collect::<String>();
            let mut index = 8;
            while index < COLUMN {
                ascii.insert_str((index - 8) / 8 * COL_SPACE.len() + index, COL_SPACE);
                index += 8;
            }
            text += &format!(" {}\n", ascii);
        }
    }
    // 最后一行单独处理格式化
    if i % COLUMN > 0 {
        for _ in 0..(COLUMN - 1 - (i - 1) % COLUMN) {
            text += "   ";
        }
        for _ in 0..(COLUMN + 8 - i % COLUMN) / 8 {
            text += COL_SPACE;
        }

        let mut ascii = arr.iter().take(((i - 1) % COLUMN) + 1).collect::<String>();
        let mut index = 8;
        let ascii_len = ascii.len();
        while index < ascii_len {
            ascii.insert_str((index - 8) / 8 * COL_SPACE.len() + index, COL_SPACE);
            index += 8;
        }
        text += &format!(" {}\n", ascii);
    }
    text
}

pub fn print_hex(bytes: &[u8]) {
    println!("{}", bytes_hex_format(bytes));
}