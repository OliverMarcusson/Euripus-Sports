use time::{Date, OffsetDateTime, PrimitiveDateTime, Time};
use time_tz::{OffsetDateTimeExt, PrimitiveDateTimeExt, Tz};

use crate::domain::EventStatus;

pub fn local_datetime(date: Date, clock: &str, timezone: &'static Tz) -> Option<OffsetDateTime> {
    let (hour, minute) = clock.split_once(':')?;
    let time = Time::from_hms(hour.parse().ok()?, minute.parse().ok()?, 0).ok()?;
    local_time(date, time, timezone)
}

pub fn local_time(date: Date, time: Time, timezone: &'static Tz) -> Option<OffsetDateTime> {
    match PrimitiveDateTime::new(date, time).assume_timezone(timezone) {
        time_tz::OffsetResult::Some(value) => Some(value),
        time_tz::OffsetResult::Ambiguous(_, _) | time_tz::OffsetResult::None => None,
    }
}

pub fn stockholm_day_bounds(now: OffsetDateTime) -> Option<(OffsetDateTime, OffsetDateTime)> {
    let timezone = time_tz::timezones::db::europe::STOCKHOLM;
    let local_date = now.to_timezone(timezone).date();
    let next_date = local_date.next_day()?;
    Some((
        local_datetime(local_date, "00:00", timezone)?,
        local_datetime(next_date, "00:00", timezone)?,
    ))
}

pub fn infer_status_at(
    now: OffsetDateTime,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> EventStatus {
    if now < start {
        EventStatus::Upcoming
    } else if now < end {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}

pub fn effective_status(
    stored: &EventStatus,
    start: OffsetDateTime,
    end: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> EventStatus {
    if *stored == EventStatus::Cancelled {
        return EventStatus::Cancelled;
    }
    if now < start {
        EventStatus::Upcoming
    } else if end.is_some_and(|end| now < end) {
        EventStatus::Live
    } else {
        EventStatus::Finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};
    use time_tz::timezones;

    #[test]
    fn stockholm_civil_time_observes_dst() {
        assert_eq!(
            local_datetime(
                date!(2026 - 02 - 28),
                "19:00",
                timezones::db::europe::STOCKHOLM
            )
            .unwrap()
            .to_offset(time::UtcOffset::UTC),
            datetime!(2026-02-28 18:00 UTC)
        );
        assert_eq!(
            local_datetime(
                date!(2026 - 07 - 01),
                "19:00",
                timezones::db::europe::STOCKHOLM
            )
            .unwrap()
            .to_offset(time::UtcOffset::UTC),
            datetime!(2026-07-01 17:00 UTC)
        );
        assert!(local_datetime(
            date!(2026 - 03 - 29),
            "02:30",
            timezones::db::europe::STOCKHOLM
        )
        .is_none());
        assert!(local_datetime(
            date!(2026 - 10 - 25),
            "02:30",
            timezones::db::europe::STOCKHOLM
        )
        .is_none());
    }

    #[test]
    fn status_is_deterministic_and_cancelled_is_sticky() {
        let start = datetime!(2026-04-01 10:00 UTC);
        let end = datetime!(2026-04-01 12:00 UTC);
        assert_eq!(
            effective_status(
                &EventStatus::Live,
                start,
                Some(end),
                datetime!(2026-04-01 09:59 UTC)
            ),
            EventStatus::Upcoming
        );
        assert_eq!(
            effective_status(&EventStatus::Upcoming, start, Some(end), start),
            EventStatus::Live
        );
        assert_eq!(
            effective_status(&EventStatus::Live, start, Some(end), end),
            EventStatus::Finished
        );
        assert_eq!(
            effective_status(&EventStatus::Cancelled, start, Some(end), start),
            EventStatus::Cancelled
        );
    }

    #[test]
    fn london_and_new_york_civil_times_observe_dst() {
        for (timezone, winter, summer) in [
            (
                timezones::db::europe::LONDON,
                datetime!(2026-01-15 19:00 UTC),
                datetime!(2026-07-15 18:00 UTC),
            ),
            (
                timezones::db::america::NEW_YORK,
                datetime!(2026-01-16 00:00 UTC),
                datetime!(2026-07-15 23:00 UTC),
            ),
        ] {
            assert_eq!(
                local_datetime(date!(2026 - 01 - 15), "19:00", timezone)
                    .unwrap()
                    .to_offset(time::UtcOffset::UTC),
                winter.to_offset(time::UtcOffset::UTC)
            );
            assert_eq!(
                local_datetime(date!(2026 - 07 - 15), "19:00", timezone)
                    .unwrap()
                    .to_offset(time::UtcOffset::UTC),
                summer.to_offset(time::UtcOffset::UTC)
            );
        }
    }

    #[test]
    fn inferred_status_has_stable_boundaries() {
        let start = datetime!(2026-04-01 10:00 UTC);
        let end = datetime!(2026-04-01 12:00 UTC);
        assert_eq!(
            infer_status_at(datetime!(2026-04-01 09:59 UTC), start, end),
            EventStatus::Upcoming
        );
        assert_eq!(infer_status_at(start, start, end), EventStatus::Live);
        assert_eq!(infer_status_at(end, start, end), EventStatus::Finished);
        assert_eq!(
            infer_status_at(datetime!(2026-04-01 12:01 UTC), start, end),
            EventStatus::Finished
        );
    }

    #[test]
    fn stockholm_day_bounds_follow_dst() {
        let (winter_start, winter_end) =
            stockholm_day_bounds(datetime!(2026-02-28 12:00 UTC)).unwrap();
        assert_eq!(winter_end - winter_start, time::Duration::hours(24));
        let (dst_start, dst_end) = stockholm_day_bounds(datetime!(2026-03-29 12:00 UTC)).unwrap();
        assert_eq!(dst_end - dst_start, time::Duration::hours(23));
    }
}
