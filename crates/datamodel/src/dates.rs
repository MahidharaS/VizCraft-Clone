//! Dates as spreadsheet serial numbers (days since 1899-12-30), for DAX date and time
//! intelligence functions.

use chrono::{Datelike, Months, NaiveDate};

fn epoch() -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(1899, 12, 30)
}

pub fn to_date(serial: f64) -> Option<NaiveDate> {
    if !serial.is_finite() || !(-700_000.0..=3_000_000.0).contains(&serial) {
        return None;
    }
    epoch()?.checked_add_signed(chrono::Duration::days(serial.floor() as i64))
}

pub fn to_serial(d: NaiveDate) -> Option<f64> {
    Some(d.signed_duration_since(epoch()?).num_days() as f64)
}

pub fn ymd(y: i32, m: u32, d: u32) -> Option<f64> {
    to_serial(NaiveDate::from_ymd_opt(y, m, d)?)
}

/// First day of the year, quarter or month (`DATESYTD`/`DATESQTD`/`DATESMTD`) holding `serial`.
pub fn period_start(serial: f64, period: &str) -> Option<f64> {
    let d = to_date(serial)?;
    match period {
        "DATESYTD" => ymd(d.year(), 1, 1),
        "DATESQTD" => ymd(d.year(), (d.month0() / 3) * 3 + 1, 1),
        _ => ymd(d.year(), d.month(), 1),
    }
}

/// The whole year, quarter or month before the one holding `serial`, as first and last day.
pub fn previous_period(serial: f64, name: &str) -> Option<(f64, f64)> {
    let d = to_date(serial)?;
    let months = match name {
        "PREVIOUSYEAR" => return Some((ymd(d.year() - 1, 1, 1)?, ymd(d.year() - 1, 12, 31)?)),
        "PREVIOUSQUARTER" => 3,
        _ => 1,
    };
    let start_this = NaiveDate::from_ymd_opt(d.year(), if months == 3 { (d.month0() / 3) * 3 + 1 } else { d.month() }, 1)?;
    let start_prev = start_this.checked_sub_months(Months::new(months))?;
    Some((to_serial(start_prev)?, to_serial(start_this)? - 1.0))
}

/// `serial` moved by `n` days, months, quarters or years (month ends clamp: Mar 31 − 1 month is
/// Feb 28/29).
pub fn shift(serial: f64, n: i32, unit: &str) -> Option<f64> {
    let d = to_date(serial)?;
    let months = match unit {
        "DAY" => return to_serial(d.checked_add_signed(chrono::Duration::days(n as i64))?),
        "MONTH" => n,
        "QUARTER" => n.checked_mul(3)?,
        "YEAR" => n.checked_mul(12)?,
        _ => return None,
    };
    let moved =
        if months >= 0 { d.checked_add_months(Months::new(months as u32))? } else { d.checked_sub_months(Months::new(months.unsigned_abs()))? };
    to_serial(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials() {
        assert_eq!(ymd(2026, 1, 1), Some(46023.0));
        assert_eq!(to_date(46023.0), NaiveDate::from_ymd_opt(2026, 1, 1));
        assert_eq!(shift(ymd(2024, 3, 31).unwrap(), -1, "MONTH"), ymd(2024, 2, 29));
        assert_eq!(previous_period(ymd(2026, 5, 10).unwrap(), "PREVIOUSQUARTER"), Some((ymd(2026, 1, 1).unwrap(), ymd(2026, 3, 31).unwrap())));
    }
}
