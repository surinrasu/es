use embedded_hal::i2c::I2c;

pub(crate) const WIDTH: usize = 128;
pub(crate) const HEIGHT: usize = 64;
pub(crate) const PAGE_N: usize = HEIGHT / 8;
pub(crate) const BUFFER_N: usize = WIDTH * PAGE_N;
pub(crate) const DEFAULT_ADDRESS: u8 = 0x3C;

const CONTROL_COMMAND: u8 = 0x00;
const CONTROL_DATA: u8 = 0x40;
const DATA_CHUNK_N: usize = 16;
const INIT_SEQUENCE: [u8; 26] = [
    0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA,
    0x12, 0x81, 0x7F, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0x2E, 0xAF,
];

pub(crate) type HsOledI2c<I2C> = HsSsd1315<I2C>;

pub(crate) struct HsSsd1315<I2C>
where
    I2C: I2c,
{
    i2c: I2C,
    address: u8,
    buffer: [u8; BUFFER_N],
}

impl<I2C> HsSsd1315<I2C>
where
    I2C: I2c,
{
    pub(crate) fn new(i2c: I2C) -> Self {
        Self::with_address(i2c, DEFAULT_ADDRESS)
    }

    pub(crate) fn with_address(i2c: I2C, address: u8) -> Self {
        Self {
            i2c,
            address,
            buffer: [0; BUFFER_N],
        }
    }

    pub(crate) fn init(&mut self) -> Result<(), I2C::Error> {
        self.write_commands(&INIT_SEQUENCE)?;
        self.clear();
        self.flush()
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.fill(0);
    }

    pub(crate) fn set_pixel(&mut self, x: usize, y: usize, enabled: bool) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }

        let index = buffer_index(x, y);
        let mask = 1u8 << (y & 0x07);

        if enabled {
            self.buffer[index] |= mask;
        } else {
            self.buffer[index] &= !mask;
        }
    }

    pub(crate) fn flush(&mut self) -> Result<(), I2C::Error> {
        self.write_commands(&[
            0x21,
            0x00,
            (WIDTH - 1) as u8,
            0x22,
            0x00,
            (PAGE_N - 1) as u8,
        ])?;

        let mut packet = [0u8; DATA_CHUNK_N + 1];
        packet[0] = CONTROL_DATA;

        for chunk in self.buffer.chunks(DATA_CHUNK_N) {
            packet[1..1 + chunk.len()].copy_from_slice(chunk);
            self.i2c.write(self.address, &packet[..1 + chunk.len()])?;
        }

        Ok(())
    }
    fn write_commands(&mut self, commands: &[u8]) -> Result<(), I2C::Error> {
        let mut packet = [0u8; DATA_CHUNK_N + 1];
        packet[0] = CONTROL_COMMAND;

        for chunk in commands.chunks(DATA_CHUNK_N) {
            packet[1..1 + chunk.len()].copy_from_slice(chunk);
            self.i2c.write(self.address, &packet[..1 + chunk.len()])?;
        }

        Ok(())
    }
}

const fn buffer_index(x: usize, y: usize) -> usize {
    x + (y / 8) * WIDTH
}
