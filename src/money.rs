/// Format a monetary amount using Vietnamese thousands separators.
pub fn format_vnd(amount: f64) -> String {
    if !amount.is_finite() {
        return amount.to_string();
    }

    let rounded = amount.round() as i128;
    let negative = rounded < 0;
    let digits = rounded.unsigned_abs().to_string();

    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            formatted.push('.');
        }
        formatted.push(ch);
    }

    if negative {
        format!("-{formatted}")
    } else {
        formatted
    }
}

#[cfg(test)]
mod tests {
    use super::format_vnd;

    #[test]
    fn formats_vietnamese_thousands_separators() {
        assert_eq!(format_vnd(0.0), "0");
        assert_eq!(format_vnd(1_000.0), "1.000");
        assert_eq!(format_vnd(1_234_567.0), "1.234.567");
        assert_eq!(format_vnd(-1_234.0), "-1.234");
    }

    #[test]
    fn rounds_fractional_amounts() {
        assert_eq!(format_vnd(1_234.6), "1.235");
    }
}
