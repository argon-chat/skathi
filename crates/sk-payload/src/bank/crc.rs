const POLYNOMIAL: u64 = 0x42f0_e1eb_a9ea_3693;

// Row n is the contribution of a byte followed by n zero bytes (slicing-by-8).
static TABLES: [[u64; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = (byte as u64) << 56;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc >> 63 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ POLYNOMIAL
            };
            bit += 1;
        }
        tables[0][byte] = crc;
        byte += 1;
    }
    let mut row = 1;
    while row < 8 {
        let mut byte = 0;
        while byte < 256 {
            let previous = tables[row - 1][byte];
            tables[row][byte] = (previous << 8) ^ tables[0][(previous >> 56) as usize];
            byte += 1;
        }
        row += 1;
    }
    tables
};

/// CRC-64/ECMA-182 over the original, uncompressed entry bytes.
pub fn crc64(bytes: &[u8]) -> u64 {
    let mut crc = 0;
    let (words, tail) = bytes.as_chunks::<8>();
    for &word in words {
        let [a, b, c, d, e, f, g, h] = (u64::from_be_bytes(word) ^ crc).to_be_bytes();
        crc = TABLES[7][a as usize]
            ^ TABLES[6][b as usize]
            ^ TABLES[5][c as usize]
            ^ TABLES[4][d as usize]
            ^ TABLES[3][e as usize]
            ^ TABLES[2][f as usize]
            ^ TABLES[1][g as usize]
            ^ TABLES[0][h as usize];
    }
    for &byte in tail {
        crc = (crc << 8) ^ TABLES[0][((crc >> 56) as u8 ^ byte) as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::{crc64, POLYNOMIAL};

    fn bitwise(bytes: &[u8]) -> u64 {
        let mut crc = 0u64;
        for &byte in bytes {
            crc ^= u64::from(byte) << 56;
            for _ in 0..8 {
                let high_bit = crc & (1 << 63) != 0;
                crc <<= 1;
                if high_bit {
                    crc ^= POLYNOMIAL;
                }
            }
        }
        crc
    }

    #[test]
    fn ecma_check_vector_and_unaligned_words_and_tails() {
        assert_eq!(crc64(b"123456789"), 0x6c40_df5f_0b49_7347);
        let bytes: Vec<_> = (0..4128usize).map(|i| (i * 183 + i / 13) as u8).collect();
        for offset in 0..8 {
            for len in (0..40).chain(4088..4104) {
                let data = &bytes[offset..offset + len];
                assert_eq!(crc64(data), bitwise(data), "offset {offset}, length {len}");
            }
        }
    }
}
