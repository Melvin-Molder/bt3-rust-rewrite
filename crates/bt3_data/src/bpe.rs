use std::io;

const HEADER_SIZE: usize = 8;
const SYMBOL_COUNT: usize = 256;
const EXPANSION_STACK_SIZE: usize = 256;

pub fn decode(source: &[u8]) -> io::Result<Vec<u8>> {
    if source.len() < HEADER_SIZE {
        return Err(invalid_data("BPE header is truncated"));
    }

    let raw_size = read_nonnegative_size(&source[0..4], "raw size")?;
    let packed_size = read_nonnegative_size(&source[4..8], "packed size")?;
    let packed_end = HEADER_SIZE
        .checked_add(packed_size)
        .filter(|end| *end <= source.len())
        .ok_or_else(|| invalid_data("BPE payload exceeds source buffer"))?;
    let mut input = Cursor::new(&source[HEADER_SIZE..packed_end]);

    let mut output = Vec::new();
    output
        .try_reserve_exact(raw_size)
        .map_err(|_| io::Error::other("could not allocate BPE output"))?;
    output.resize(raw_size, 0);
    let mut output_position = 0;

    while input.position < input.bytes.len() {
        let mut left = [0u8; SYMBOL_COUNT];
        let mut right = [0u8; SYMBOL_COUNT];
        for (symbol, value) in left.iter_mut().enumerate() {
            *value = symbol as u8;
        }

        let mut next_symbol = 0usize;
        let mut run = input.read_byte()?;
        loop {
            if run > 127 {
                next_symbol = next_symbol
                    .checked_add(usize::from(run - 127))
                    .filter(|next| *next <= SYMBOL_COUNT)
                    .ok_or_else(|| invalid_data("BPE symbol table run exceeds table"))?;
                run = 0;
            }
            if next_symbol == SYMBOL_COUNT {
                break;
            }

            for _ in 0..=run {
                if next_symbol >= SYMBOL_COUNT {
                    return Err(invalid_data("BPE symbol table exceeds 256 entries"));
                }
                let first = input.read_byte()?;
                left[next_symbol] = first;
                if first != next_symbol as u8 {
                    right[next_symbol] = input.read_byte()?;
                }
                next_symbol += 1;
            }

            if next_symbol == SYMBOL_COUNT {
                break;
            }
            run = input.read_byte()?;
        }

        let encoded_count = input.read_be_u16()?;
        for _ in 0..encoded_count {
            let symbol = input.read_byte()?;
            expand_symbol(symbol, &left, &right, &mut output, &mut output_position)?;
        }
    }

    if output_position != raw_size {
        return Err(invalid_data("BPE output size does not match its header"));
    }

    Ok(output)
}

fn expand_symbol(
    initial: u8,
    left: &[u8; SYMBOL_COUNT],
    right: &[u8; SYMBOL_COUNT],
    output: &mut [u8],
    output_position: &mut usize,
) -> io::Result<()> {
    let mut pending = [0u8; EXPANSION_STACK_SIZE];
    let mut pending_count = 0usize;
    let mut symbol = initial;

    loop {
        let index = usize::from(symbol);
        if left[index] == symbol {
            if *output_position == output.len() {
                return Err(invalid_data(
                    "BPE data expands beyond its declared raw size",
                ));
            }
            output[*output_position] = symbol;
            *output_position += 1;

            if pending_count == 0 {
                return Ok(());
            }
            pending_count -= 1;
            symbol = pending[pending_count];
        } else {
            if pending_count + 2 > pending.len() {
                return Err(invalid_data("BPE symbol expansion exceeds stack capacity"));
            }
            pending[pending_count] = right[index];
            pending[pending_count + 1] = left[index];
            pending_count += 2;
            pending_count -= 1;
            symbol = pending[pending_count];
        }
    }
}

fn read_nonnegative_size(bytes: &[u8], field: &str) -> io::Result<usize> {
    let value = i32::from_le_bytes(bytes.try_into().unwrap());
    usize::try_from(value).map_err(|_| invalid_data(&format!("BPE {field} is negative")))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        let byte = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or_else(|| invalid_data("BPE block is truncated"))?;
        self.position += 1;
        Ok(byte)
    }

    fn read_be_u16(&mut self) -> io::Result<u16> {
        let high = self.read_byte()?;
        let low = self.read_byte()?;
        Ok(u16::from_be_bytes([high, low]))
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_literal_symbols_from_a_block() {
        let mut packed = vec![127];
        packed.extend(0..=127);
        packed.extend([255, 0, 3, b'A', b'B', b'C']);

        let decoded = decode(&make_bpe(3, &packed)).unwrap();

        assert_eq!(decoded, b"ABC");
    }

    #[test]
    fn expands_a_pair_in_left_to_right_order() {
        let mut packed = vec![0, b'A', b'B', 126];
        packed.extend(1..=127);
        packed.extend([255, 0, 1, 0]);

        let decoded = decode(&make_bpe(2, &packed)).unwrap();

        assert_eq!(decoded, b"AB");
    }

    #[test]
    fn rejects_output_that_disagrees_with_header() {
        let mut packed = vec![127];
        packed.extend(0..=127);
        packed.extend([255, 0, 2, b'A', b'B']);

        assert!(decode(&make_bpe(3, &packed)).is_err());
    }

    fn make_bpe(raw_size: u32, packed: &[u8]) -> Vec<u8> {
        let mut source = Vec::with_capacity(8 + packed.len());
        source.extend(raw_size.to_le_bytes());
        source.extend((packed.len() as u32).to_le_bytes());
        source.extend(packed);
        source
    }
}
