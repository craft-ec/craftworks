//! GF(2^8), the field RLNC codes over (the AES polynomial x⁸+x⁴+x³+x+1 is not used: 0x11D, the Reed-Solomon one,
//! whose generator 2 is primitive): add is XOR, multiply by log/exp tables.

const POLY: u16 = 0x11D;

pub struct Tables {
    exp: [u8; 512],
    log: [u8; 256],
}

pub const fn tables() -> Tables {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= POLY;
        }
        i += 1;
    }
    let mut j = 255;
    while j < 512 {
        exp[j] = exp[j - 255];
        j += 1;
    }
    Tables { exp, log }
}

pub static T: Tables = tables();

pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        T.exp[T.log[a as usize] as usize + T.log[b as usize] as usize]
    }
}

pub fn inv(a: u8) -> u8 {
    assert!(a != 0, "no inverse of 0");
    T.exp[255 - T.log[a as usize] as usize]
}

/// `dst += c · src`, byte by byte (the row operation every step of coding and decoding is).
pub fn axpy(dst: &mut [u8], src: &[u8], c: u8) {
    match c {
        0 => {}
        1 => dst.iter_mut().zip(src).for_each(|(d, s)| *d ^= s),
        _ => {
            // One row of the multiplication table for `c`: a lookup per byte.
            let mut row = [0u8; 256];
            for (v, r) in row.iter_mut().enumerate() {
                *r = mul(c, v as u8);
            }
            dst.iter_mut().zip(src).for_each(|(d, s)| *d ^= row[*s as usize]);
        }
    }
}

/// `v *= c`.
pub fn scale(v: &mut [u8], c: u8) {
    if c == 1 {
        return;
    }
    let mut row = [0u8; 256];
    for (x, r) in row.iter_mut().enumerate() {
        *r = mul(c, x as u8);
    }
    v.iter_mut().for_each(|b| *b = row[*b as usize]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_field_s_laws_hold() {
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1);
            assert_eq!(mul(a, 1), a);
            assert_eq!(mul(a, 0), 0);
        }
        // Distributes over addition, commutes, associates (spot checks across the table).
        for (a, b, c) in [(3u8, 7u8, 200u8), (0x53, 0xCA, 0x11), (255, 254, 2)] {
            assert_eq!(mul(a, b ^ c), mul(a, b) ^ mul(a, c));
            assert_eq!(mul(a, b), mul(b, a));
            assert_eq!(mul(mul(a, b), c), mul(a, mul(b, c)));
        }
        // 2 generates the whole group (the polynomial is primitive).
        let mut seen = std::collections::HashSet::new();
        let mut x = 1u8;
        for _ in 0..255 {
            seen.insert(x);
            x = mul(x, 2);
        }
        assert_eq!(seen.len(), 255);
    }
}
