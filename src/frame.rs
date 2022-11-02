use crate::*;

pub trait Frame {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>);
}

impl<T: Frame + ?Sized> Frame for &T {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        T::encode::<SIDE>(self, writer)
    }
}

impl<T: Frame + ?Sized> Frame for Box<T> {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        T::encode::<SIDE>(self, writer)
    }
}

impl Frame for str {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        encode::<SIDE, RandMask>(writer, true, 1, self.as_bytes());
    }
}

impl Frame for [u8] {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        encode::<SIDE, RandMask>(writer, true, 2, self);
    }
}

impl<const N: usize> Frame for [u8; N] {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        encode::<SIDE, RandMask>(writer, true, 2, self);
    }
}

impl Frame for Event<'_> {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        match self {
            Event::Ping(data) => encode::<SIDE, RandMask>(writer, true, 9, data),
            Event::Pong(data) => encode::<SIDE, RandMask>(writer, true, 10, data),
        }
    }
}

pub(crate) struct Close<'a> { pub code: u16, pub reason: &'a [u8] }
impl<'a> Frame for Close<'a> {
    fn encode<const SIDE: bool>(&self, writer: &mut Vec<u8>) {
        let mut data = Vec::with_capacity(2 + self.reason.len());
        data.extend_from_slice(&self.code.to_be_bytes());
        data.extend_from_slice(self.reason);
        frame::encode::<SIDE, frame::RandMask>(writer, true, 8, &data);
    }
}

pub fn encode<const SIDE: bool, Mask: RandKey>(
    writer: &mut Vec<u8>,
    fin: bool,
    opcode: u8,
    data: &[u8],
) {
    let data_len = data.len();
    writer.reserve(if SERVER == SIDE { 10 } else { 14 } + data_len);
    unsafe {
        let filled = writer.len();
        let start = writer.as_mut_ptr().add(filled);

        let mask_bit = if SERVER == SIDE { 0 } else { 0x80 };

        start.write(((fin as u8) << 7) | opcode);
        let len = if data_len < 126 {
            start.add(1).write(mask_bit | data_len as u8);
            2
        } else if data_len < 65536 {
            let [b2, b3] = (data_len as u16).to_be_bytes();
            start.add(1).write(mask_bit | 126);
            start.add(2).write(b2);
            start.add(3).write(b3);
            4
        } else {
            let [b2, b3, b4, b5, b6, b7, b8, b9] = (data_len as u64).to_be_bytes();
            start.add(1).write(mask_bit | 127);
            start.add(2).write(b2);
            start.add(3).write(b3);
            start.add(4).write(b4);
            start.add(5).write(b5);
            start.add(6).write(b6);
            start.add(7).write(b7);
            start.add(8).write(b8);
            start.add(9).write(b9);
            10
        };

        let header_len = if SERVER == SIDE {
            std::ptr::copy_nonoverlapping(data.as_ptr(), start.add(len), data_len);
            len
        } else {
            let mask = Mask::key();
            let [a, b, c, d] = mask;
            start.add(len).write(a);
            start.add(len + 1).write(b);
            start.add(len + 2).write(c);
            start.add(len + 3).write(d);

            let dist = start.add(len + 4);
            for (index, byte) in data.iter().enumerate() {
                dist.add(index).write(byte ^ mask.get_unchecked(index % 4));
            }
            len + 4
        };
        writer.set_len(filled + header_len + data_len);
    }
}

/// Default random mask generator
pub struct RandMask;

pub trait RandKey {
    fn key() -> [u8; 4];
}

impl RandKey for RandMask {
    fn key() -> [u8; 4] {
        fastrand::u32(..).to_ne_bytes()
    }
}

#[cfg(test)]
mod encode {
    use super::*;
    const DATA: &[u8] = b"Hello";

    struct DefaultMask;
    impl super::RandKey for DefaultMask {
        fn key() -> [u8; 4] {
            [55, 250, 33, 61]
        }
    }

    fn encode<const S: bool>(writer: &mut Vec<u8>, fin: bool, opcode: u8, data: &[u8]) {
        super::encode::<S, DefaultMask>(writer, fin, opcode, data);
    }

    #[test]
    fn unmasked_txt_msg() {
        let mut bytes = vec![];
        encode::<SERVER>(&mut bytes, true, 1, DATA);
        assert_eq!(bytes, [0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]);
    }

    #[test]
    fn masked_txt_msg() {
        let mut bytes = vec![];
        encode::<CLIENT>(&mut bytes, true, 1, DATA);
        assert_eq!(
            bytes,
            [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]
        );
    }

    #[test]
    fn fragmented_unmasked_txt_msg() {
        let mut bytes = vec![];
        encode::<SERVER>(&mut bytes, false, 1, b"Hel");
        encode::<SERVER>(&mut bytes, true, 0, b"lo");
        assert_eq!(
            bytes,
            [
                0x01, 0x03, 0x48, 0x65, 0x6c, // fragmented frame
                0x80, 0x02, 0x6c, 0x6f, // final frame
            ]
        );
    }

    #[test]
    fn unmasked_ping_req_and_masked_pong_res() {
        let mut bytes = vec![];
        encode::<SERVER>(&mut bytes, true, 9, DATA);
        encode::<CLIENT>(&mut bytes, true, 10, DATA);
        assert_eq!(
            bytes,
            [
                // unmasked ping request
                0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f, //
                // masked pong response
                0x8a, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
            ]
        );
    }
}
