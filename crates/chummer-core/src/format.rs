//! Display formatting shared by front ends.

/// Nuyen as `#,0.##¥`, Chummer's default `nuyenformat`.
pub fn nuyen(v: f64) -> String {
    let neg = v < 0.0;
    let cents = (v.abs() * 100.0).round() as u64;
    let (whole, frac) = (cents / 100, cents % 100);
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let frac = match frac {
        0 => String::new(),
        f if f % 10 == 0 => format!(".{}", f / 10),
        f => format!(".{f:02}"),
    };
    format!("{}{grouped}{frac}¥", if neg { "-" } else { "" })
}

/// Essence with a fixed number of decimals.
pub fn essence(v: f64, decimals: u32) -> String {
    format!("{:.*}", decimals as usize, v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn nuyen_format() {
        assert_eq!(super::nuyen(756.333333), "756.33¥");
        assert_eq!(super::nuyen(1234567.0), "1,234,567¥");
        assert_eq!(super::nuyen(-50.5), "-50.5¥");
        assert_eq!(super::nuyen(0.0), "0¥");
    }
}
