//! Canonical numeric literal values, including integers beyond machine width.
use super::{Forms, LitValue};

pub(super) fn decode(source: &str, negative: bool, forms: &Forms) -> Option<LitValue> {
    // Bound ingestion work on adversarial generated literals; never approximate.
    if source.len() > 16_384 {
        return None;
    }
    let text: String = source
        .chars()
        .filter(|&c| c != forms.numeric_separator)
        .collect();
    let bigint = text.strip_suffix(forms.bigint_suffix);
    let digits = bigint.unwrap_or(&text);
    let (digits, radix) = forms
        .radix_prefixes
        .iter()
        .find_map(|&(prefix, radix)| digits.strip_prefix(prefix).map(|digits| (digits, radix)))
        .unwrap_or((digits, 10));
    if bigint.is_some() || radix != 10 {
        let words = integer(digits, radix)?;
        if bigint.is_some() {
            return Some(LitValue::BigInt {
                negative: negative && !words.is_empty(),
                words,
            });
        }
        return number(as_float(&words), negative);
    }
    number(digits.parse::<f64>().ok()?, negative)
}

fn number(value: f64, negative: bool) -> Option<LitValue> {
    if value.is_nan() || value < 0.0 {
        return None;
    }
    let value = if value == 0.0 {
        0.0
    } else if negative {
        -value
    } else {
        value
    };
    Some(LitValue::Number(value.to_bits()))
}

fn integer(digits: &str, radix: u32) -> Option<Vec<u32>> {
    if digits.is_empty() || !(2..=36).contains(&radix) {
        return None;
    }
    let mut words = Vec::<u32>::new();
    for ch in digits.chars() {
        let mut carry = u64::from(ch.to_digit(radix)?);
        for word in &mut words {
            let next = u64::from(*word) * u64::from(radix) + carry;
            *word = next as u32;
            carry = next >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
    }
    Some(words)
}

fn as_float(words: &[u32]) -> f64 {
    let Some(&last) = words.last() else {
        return 0.0;
    };
    let bits = (words.len() - 1) * 32 + (32 - last.leading_zeros()) as usize;
    let bit = |index: usize| (words[index / 32] >> (index % 32)) & 1;
    let shift = bits.saturating_sub(53);
    let mut mantissa = 0u64;
    for index in (shift..bits).rev() {
        mantissa = (mantissa << 1) | u64::from(bit(index));
    }
    if shift > 0
        && bit(shift - 1) != 0
        && (mantissa & 1 != 0 || (0..shift - 1).any(|index| bit(index) != 0))
    {
        mantissa += 1;
    }
    (mantissa as f64) * 2.0f64.powi(shift.try_into().unwrap_or(i32::MAX))
}

#[cfg(test)]
#[path = "lexical_atomic_numbers_tests.rs"]
mod tests;
