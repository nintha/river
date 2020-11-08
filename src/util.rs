use chrono::Local;
use std::io::Write;
use std::future::Future;
use rand::Rng;
use std::fmt::Debug;

pub fn init_logger() {
    let env = env_logger::Env::default()
        .filter_or(env_logger::DEFAULT_FILTER_ENV, "info");
    // 设置日志打印格式
    env_logger::Builder::from_env(env)
        .format(|buf, record| {
            writeln!(
                buf,
                "{} {} - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                buf.default_styled_level(record.level()),
                &record.args()
            )
        })
        .init();
    log::info!("env_logger initialized.");
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

/// 执行一个新协程，并且在错误时打印错误信息
pub fn spawn_and_log_error<F, E>(fut: F)
    where
        F: Future<Output=Result<(), E>> + Send + 'static,
        E: Debug,
{
    smol::spawn(async move {
        if let Err(e) = fut.await {
            log::error!("spawn future error, {:?}", e)
        }
    }).detach();
}

/// 生成随机字节数组
pub fn gen_random_bytes(len: u32) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut vec = Vec::new();
    for _ in 0..len {
        vec.push(rng.gen());
    }
    vec
}