use std::io::{BufReader, BufWriter, Cursor, Read, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const END: u8 = 192;
const ESC: u8 = 219;
const ESC_END: u8 = 220;
const ESC_ESC: u8 = 221;

pub struct Reader<R>
where
    R: Read,
{
    inner: BufReader<R>,
}

impl<R> Reader<R>
where
    R: Read,
{
    pub fn new(read: R) -> Self {
        Self {
            inner: BufReader::new(read),
        }
    }

    fn read_byte(&mut self) -> Result<u8> {
        let mut buf = [0; 1];
        self.inner.read(&mut buf)?;
        Ok(buf[0])
    }

    fn check_and_remove_crc(&self, frame: Vec<u8>) -> Result<Vec<u8>> {
        let len = frame.len() - 2;

        // Check CRC16 matches:
        let mut cursor = Cursor::new(&frame[len..]);
        let provided_crc = cursor.read_u16::<LittleEndian>()?;
        let calculated_crc = crc16(&frame[..len]);
        assert_eq!(provided_crc, calculated_crc); // TODO: return error

        // Remove CRC16 bytes from returned frame:
        let mut frame = frame;
        frame.truncate(len);

        Ok(frame)
    }

    pub fn read_frame(&mut self) -> Result<Vec<u8>> {
        let mut frame = Vec::new();
        loop {
            let mut byte = self.read_byte()?;

            if byte == END {
                // Skip END bytes at start of frame.
                if frame.is_empty() {
                    continue;
                }

                let frame = self.check_and_remove_crc(frame)?;
                return Ok(frame);
            }

            if byte == ESC {
                byte = match self.read_byte()? {
                    ESC_ESC => ESC,
                    ESC_END => END,
                    _ => panic!(), // TODO: return error
                }
            }

            frame.push(byte);
        }
    }
}

pub struct Writer<W>
where
    W: Write,
{
    inner: BufWriter<W>,
}

impl<W> Writer<W>
where
    W: Write,
{
    pub fn new(write: W) -> Self {
        Self {
            inner: BufWriter::new(write),
        }
    }

    fn write_byte(&mut self, byte: u8) -> Result<()> {
        Ok(self.inner.write_u8(byte)?)
    }

    fn write_crc(&mut self, data: &[u8]) -> Result<()> {
        let crc = crc16(data);
        self.inner.write_u16::<LittleEndian>(crc)?;
        Ok(())
    }

    pub fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        self.write_byte(END)?;
        for byte in data {
            match *byte {
                ESC => {
                    self.write_byte(ESC)?;
                    self.write_byte(ESC_ESC)?;
                }
                END => {
                    self.write_byte(ESC)?;
                    self.write_byte(ESC_END)?;
                }
                byte => self.write_byte(byte)?,
            };
        }
        self.write_crc(data)?;
        self.write_byte(END)?;
        self.inner.flush()?;
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
