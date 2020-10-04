use std::convert::TryInto;
use std::fmt::{self, Display};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};

use crate::Result;

const END: u8 = 192;
const ESC: u8 = 219;
const ESC_END: u8 = 220;
const ESC_ESC: u8 = 221;

#[derive(Clone, Copy, Debug)]
pub enum SlipError {
    MissingCrc,
    MismatchedCrc,
    InvalidEscape,
}

impl Display for SlipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlipError::MissingCrc => write!(f, "missing CRC"),
            SlipError::MismatchedCrc => write!(f, "mismatched CRC"),
            SlipError::InvalidEscape => write!(f, "invalid escape sequence"),
        }
    }
}

impl std::error::Error for SlipError {}

pub struct Reader<R>
where
    R: AsyncRead + Unpin,
{
    inner: BufReader<R>,
}

impl<R> Reader<R>
where
    R: AsyncRead + Unpin,
{
    pub fn new(read: R) -> Self {
        Self {
            inner: BufReader::new(read),
        }
    }

    async fn read_byte(&mut self) -> Result<u8> {
        let mut buf = [0; 1];
        self.inner.read(&mut buf).await?;
        Ok(buf[0])
    }

    fn check_and_remove_crc(&self, frame: Vec<u8>) -> Result<Vec<u8>> {
        let len = frame.len() - 2;

        // Check CRC16 matches:
        let bytes = (&frame[len..])
            .try_into()
            .map_err(|_| SlipError::MissingCrc)?;
        let provided_crc = u16::from_le_bytes(bytes);
        let calculated_crc = crc16(&frame[..len]);

        if provided_crc != calculated_crc {
            return Err(SlipError::MismatchedCrc.into());
        }

        // Remove CRC16 bytes from returned frame:
        let mut frame = frame;
        frame.truncate(len);

        Ok(frame)
    }

    pub async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let mut frame = Vec::new();
        loop {
            let mut byte = self.read_byte().await?;

            if byte == END {
                // Skip END bytes at start of frame.
                if frame.is_empty() {
                    continue;
                }

                let frame = self.check_and_remove_crc(frame)?;
                return Ok(frame);
            }

            if byte == ESC {
                byte = match self.read_byte().await? {
                    ESC_ESC => ESC,
                    ESC_END => END,
                    _ => return Err(SlipError::InvalidEscape.into()),
                }
            }

            frame.push(byte);
        }
    }
}

pub struct Writer<W>
where
    W: AsyncWrite + Unpin,
{
    inner: BufWriter<W>,
}

impl<W> Writer<W>
where
    W: AsyncWrite + Unpin,
{
    pub fn new(write: W) -> Self {
        Self {
            inner: BufWriter::new(write),
        }
    }

    async fn write_byte(&mut self, byte: u8) -> Result<()> {
        Ok(self.inner.write_u8(byte).await?)
    }

    async fn write_crc(&mut self, data: &[u8]) -> Result<()> {
        let crc = crc16(data);
        self.inner.write_u16_le(crc).await?;
        Ok(())
    }

    pub async fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        self.write_byte(END).await?;
        for byte in data {
            match *byte {
                ESC => {
                    self.write_byte(ESC).await?;
                    self.write_byte(ESC_ESC).await?;
                }
                END => {
                    self.write_byte(ESC).await?;
                    self.write_byte(ESC_END).await?;
                }
                byte => self.write_byte(byte).await?,
            };
        }
        self.write_crc(data).await?;
        self.write_byte(END).await?;
        self.inner.flush().await?;
        Ok(())
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0;
    for byte in data {
        crc += u16::from(*byte);
    }
    crc = !crc + 1;
    crc
}
