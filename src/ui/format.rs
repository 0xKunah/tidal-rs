//! Display formatting for durations and dates.

use std::time::Duration;

/// `4:03`, or `1:02:05` past an hour.
pub fn clock(duration: Duration) -> String {
	let seconds = duration.as_secs();
	let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
	if hours > 0 {
		format!("{hours}:{minutes:02}:{seconds:02}")
	} else {
		format!("{minutes}:{seconds:02}")
	}
}

/// `2 h 51 min`, or `25 min` under an hour.
pub fn total(duration: Duration) -> String {
	let minutes = duration.as_secs() / 60;
	let (hours, minutes) = (minutes / 60, minutes % 60);
	if hours > 0 {
		format!("{hours} h {minutes:02} min")
	} else {
		format!("{minutes} min")
	}
}

/// `12/03/2026` from an ISO 8601 timestamp; empty when it cannot be read.
pub fn added_date(iso: &str) -> String {
	let date = iso.get(..10).unwrap_or_default();
	let parts: Vec<&str> = date.split('-').collect();
	match parts[..] {
		[year, month, day]
			if year.len() == 4
				&& month.len() == 2
				&& day.len() == 2
				&& date.chars().filter(char::is_ascii_digit).count() == 8 =>
		{
			format!("{day}/{month}/{year}")
		}
		_ => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;

	#[test]
	fn track_durations() {
		assert_eq!(clock(Duration::from_secs(243)), "4:03");
		assert_eq!(clock(Duration::from_secs(59)), "0:59");
		assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
	}

	#[test]
	fn total_durations() {
		assert_eq!(total(Duration::from_secs(10260)), "2 h 51 min");
		assert_eq!(total(Duration::from_secs(1500)), "25 min");
	}

	#[test]
	fn added_dates() {
		assert_eq!(added_date("2026-03-12T10:00:00.000+0000"), "12/03/2026");
		assert_eq!(added_date("garbage"), "");
	}
}
