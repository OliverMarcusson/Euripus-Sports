use time::{Date, OffsetDateTime};
use time_tz::Tz;

pub(crate) fn parse_clock_in_timezone(
    date: Date,
    value: &str,
    timezone: &'static Tz,
) -> Option<OffsetDateTime> {
    crate::time_utils::local_datetime(date, value, timezone)
}

pub mod allsvenskan;
pub mod champions_league;
pub mod damallsvenskan;
pub mod elitettan;
pub mod elitserien;
pub mod formula1;
pub mod hockeyallsvenskan;
pub mod listing_time;
pub mod loader;
pub mod lpga_tour;
pub mod ndhl;
pub mod pga_tour;
pub mod premier_league;
pub mod sdhl;
pub mod shl;
pub mod superettan;
pub mod svenskfotboll_league;
pub mod tv4play;
pub mod viaplay;
pub mod world_cup;

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn parse_clock_accepts_valid_boundaries_and_rejects_malformed_values() {
        for value in ["00:00", "23:59"] {
            assert!(parse_clock_in_timezone(
                date!(2026 - 04 - 01),
                value,
                time_tz::timezones::db::europe::STOCKHOLM,
            )
            .is_some());
        }
        for value in [
            "24:00",
            "23:60",
            "99:99",
            "12",
            "999:00",
            "12:999",
            "ab:cd",
            "１２:００",
        ] {
            assert!(
                parse_clock_in_timezone(
                    date!(2026 - 04 - 01),
                    value,
                    time_tz::timezones::db::europe::STOCKHOLM,
                )
                .is_none(),
                "unexpectedly accepted {value}"
            );
        }
    }
}
