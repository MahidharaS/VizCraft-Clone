//! Number and date formatting with Excel/Power BI-style format strings.

use chrono::{Datelike, NaiveDate, Timelike};

/// General format: up to 10 significant digits, no trailing zeros.
pub fn general(n: f64) -> String {
    if !n.is_finite() {
        return if n.is_nan() { "NaN".into() } else if n > 0.0 { "∞".into() } else { "-∞".into() };
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{:.10}", n);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Thousands separators.
fn group(int: &str) -> String {
    let (neg, digits) = match int.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, int),
    };
    let mut out = String::new();
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if neg { format!("-{out}") } else { out }
}

/// Compact display units: 1.2K, 3.4M, 5.6bn.
pub fn compact(n: f64, decimals: usize) -> String {
    let a = n.abs();
    let (v, suf) = if a >= 1e12 {
        (n / 1e12, "T")
    } else if a >= 1e9 {
        (n / 1e9, "bn")
    } else if a >= 1e6 {
        (n / 1e6, "M")
    } else if a >= 1e3 {
        (n / 1e3, "K")
    } else {
        (n, "")
    };
    let s = format!("{:.*}", decimals, v);
    let s = if decimals > 0 && suf.is_empty() && v == v.trunc() { format!("{}", v as i64) } else { s };
    format!("{s}{suf}")
}

/// Formats `n` with a format string. Supports `#,0`, `#,0.00`, `0.0%`, `$#,0`, `€#,0.0`,
/// `0.00E+00`, `K`/`M` display units (`#,0.0,,"M"` style via the `auto`/`compact` keywords),
/// and date formats when the string contains `y`, `m`/`M`, `d`, `h`, `s`.
pub fn format(n: f64, fmt: &str) -> String {
    let f = fmt.trim();
    if f.is_empty() || f.eq_ignore_ascii_case("general") {
        return general(n);
    }
    if f.eq_ignore_ascii_case("auto") || f.eq_ignore_ascii_case("compact") {
        return compact(n, 1);
    }
    if is_date_format(f) {
        return format_date(n, f);
    }
    // Sections: positive;negative;zero
    let sections: Vec<&str> = f.split(';').collect();
    let (sec, n_abs_neg) = if n < 0.0 && sections.len() > 1 {
        (sections.get(1).copied().unwrap_or(f), true)
    } else if n == 0.0 && sections.len() > 2 {
        (sections.get(2).copied().unwrap_or(f), false)
    } else {
        (sections.first().copied().unwrap_or(f), false)
    };
    let value = if n_abs_neg { n.abs() } else { n };
    format_section(value, sec)
}

fn format_section(n: f64, sec: &str) -> String {
    // Split prefix / numeric core / suffix.
    let core_start = sec.find(|c: char| c == '#' || c == '0' || c == '.' || c == ',');
    let Some(start) = core_start else { return sec.replace('"', "") };
    let end = sec.rfind(|c: char| c == '#' || c == '0' || c == '.' || c == ',' || c == '%').map(|i| i + 1).unwrap_or(sec.len());
    let prefix = sec.get(..start).unwrap_or("").replace('"', "");
    let core = sec.get(start..end).unwrap_or("");
    let suffix = sec.get(end..).unwrap_or("").replace('"', "");
    let percent = core.contains('%') || suffix.contains('%');
    let mut v = if percent { n * 100.0 } else { n };
    let core_np = core.trim_end_matches('%');
    // Trailing commas scale by 1000 each.
    let mut c = core_np.to_string();
    while c.ends_with(',') {
        c.pop();
        v /= 1000.0;
    }
    let sci = suffix.starts_with("E+") || suffix.starts_with("e+");
    if sci {
        let decimals = c.split('.').nth(1).map(|d| d.len()).unwrap_or(0);
        return format!("{prefix}{:.*e}", decimals, v);
    }
    let decimals = c.split('.').nth(1).map(|d| d.chars().filter(|ch| *ch == '0' || *ch == '#').count()).unwrap_or(0);
    let min_dec = c.split('.').nth(1).map(|d| d.chars().filter(|ch| *ch == '0').count()).unwrap_or(0);
    let grouping = c.contains(',');
    let mut s = format!("{:.*}", decimals, v);
    if decimals > min_dec && s.contains('.') {
        while s.ends_with('0') && s.split('.').nth(1).map(|d| d.len()).unwrap_or(0) > min_dec {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let int = if grouping { group(&int) } else { int };
    let body = match frac {
        Some(f) => format!("{int}.{f}"),
        None => int,
    };
    let pct = if percent && !suffix.contains('%') { "%" } else { "" };
    // Put a minus sign before the currency symbol.
    if let Some(b) = body.strip_prefix('-') { format!("-{prefix}{b}{pct}{suffix}") } else { format!("{prefix}{body}{pct}{suffix}") }
}

pub fn is_date_format(f: &str) -> bool {
    let l = f.to_ascii_lowercase();
    (l.contains("yy") || l.contains("dd") || l.contains("mmm") || l.contains("hh") || l.contains("d/") || l.contains("m/"))
        && !l.contains('#')
        && !l.contains('0')
}

/// Serial date (days since 1899-12-30) to text.
pub fn format_date(serial: f64, f: &str) -> String {
    let Some(base) = NaiveDate::from_ymd_opt(1899, 12, 30) else { return general(serial) };
    let days = serial.floor();
    if !days.is_finite() || days.abs() > 3_000_000.0 {
        return general(serial);
    }
    let Some(date) = base.checked_add_signed(chrono::Duration::days(days as i64)) else { return general(serial) };
    let secs = ((serial - days) * 86400.0).round() as u32;
    let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs.min(86399), 0).unwrap_or_default();
    let dt = date.and_time(time);
    let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let days_n = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let mut out = String::new();
    let chars: Vec<char> = f.chars().collect();
    let mut i = 0;
    let mut last_was_hour = false;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        let mut run = 1;
        while chars.get(i + run) == Some(&c) {
            run += 1;
        }
        let lc = c.to_ascii_lowercase();
        let month_name = months.get(dt.month0() as usize).copied().unwrap_or("");
        let day_name = days_n.get(dt.weekday().num_days_from_monday() as usize).copied().unwrap_or("");
        match lc {
            'y' => {
                out += &if run <= 2 { format!("{:02}", dt.year() % 100) } else { format!("{}", dt.year()) };
                last_was_hour = false;
            }
            'm' if c == 'M' || !last_was_hour => {
                out += &match run {
                    1 => format!("{}", dt.month()),
                    2 => format!("{:02}", dt.month()),
                    3 => month_name.get(..3).unwrap_or("").to_string(),
                    _ => month_name.to_string(),
                };
            }
            'm' => {
                out += &format!("{:02}", dt.minute());
            }
            'd' => {
                out += &match run {
                    1 => format!("{}", dt.day()),
                    2 => format!("{:02}", dt.day()),
                    3 => day_name.get(..3).unwrap_or("").to_string(),
                    _ => day_name.to_string(),
                };
                last_was_hour = false;
            }
            'h' => {
                out += &if run >= 2 { format!("{:02}", dt.hour()) } else { format!("{}", dt.hour()) };
                last_was_hour = true;
            }
            's' => out += &format!("{:02}", dt.second()),
            'q' => out += &format!("{}", dt.month0() / 3 + 1),
            _ => {
                for _ in 0..run {
                    out.push(c);
                }
            }
        }
        i += run;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats() {
        assert_eq!(format(1234567.891, "#,0"), "1,234,568");
        assert_eq!(format(1234.5, "$#,0.00"), "$1,234.50");
        assert_eq!(format(0.1234, "0.0%"), "12.3%");
        assert_eq!(format(-5.0, "#,0"), "-5");
        assert_eq!(format(1500000.0, "auto"), "1.5M");
        assert_eq!(format(45292.0, "yyyy-mm-dd"), "2024-01-01");
        assert_eq!(format(45292.0, "mmm yyyy"), "Jan 2024");
        assert_eq!(general(2.5), "2.5");
    }
}
