// SPDX-License-Identifier: AGPL-3.0-or-later

//! Human-readable durations, counts and dates for cards and the player.

use flaneur_model::{Item, ItemKind, Precision, Published, Timestamp};

/// `m:ss` under an hour, `h:mm:ss` above.
pub fn duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A compact count: `999`, `1.2K`, `3.4M`, `1.1B`.
pub fn compact(count: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];
    for (size, suffix) in UNITS {
        if count >= size {
            let tenths = count / (size / 10);
            return if tenths >= 100 || tenths.is_multiple_of(10) {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            };
        }
    }
    count.to_string()
}

/// `1 view`, `1.2K views`.
pub fn views(count: u64) -> String {
    if count == 1 {
        "1 view".into()
    } else {
        format!("{} views", compact(count))
    }
}

/// How long ago something was published, no more precise than the service was.
pub fn published(published: &Published, now: Timestamp) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let ago = (now - published.at).max(0);
    if published.precision != Precision::Exact && ago < DAY {
        return "today".into();
    }
    let (amount, unit) = match ago {
        a if a < MINUTE => return "just now".into(),
        a if a < HOUR => (a / MINUTE, "minute"),
        a if a < DAY => (a / HOUR, "hour"),
        a if a < 7 * DAY => (a / DAY, "day"),
        a if a < 30 * DAY => (a / (7 * DAY), "week"),
        a if a < 365 * DAY => (a / (30 * DAY), "month"),
        a => (a / (365 * DAY), "year"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    format!("{amount} {unit}{plural} ago")
}

/// The badge over a thumbnail: the duration, or the kind for lives and upcoming items.
pub fn badge(item: &Item) -> Option<String> {
    match item.kind {
        ItemKind::Live => Some("LIVE".into()),
        ItemKind::Upcoming => Some("UPCOMING".into()),
        ItemKind::Video | ItemKind::Short | ItemKind::Premiere => {
            item.duration.map(|d| duration(d.into()))
        }
    }
}

/// The line under the channel name: views and publication, whichever are known.
pub fn meta(item: &Item, now: Timestamp) -> String {
    let views = item.views.map(|v| {
        if item.kind == ItemKind::Live {
            format!("{} watching", compact(v))
        } else {
            views(v)
        }
    });
    let published = item.published.as_ref().map(|p| published(p, now));
    [views, published]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(65), "1:05");
        assert_eq!(duration(3_600), "1:00:00");
        assert_eq!(duration(3_725), "1:02:05");
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1K");
        assert_eq!(compact(1_234), "1.2K");
        assert_eq!(compact(56_789), "56K");
        assert_eq!(compact(3_400_000), "3.4M");
        assert_eq!(compact(1_100_000_000), "1.1B");
        assert_eq!(views(1), "1 view");
        assert_eq!(views(0), "0 views");
    }

    #[test]
    fn published_respects_precision() {
        let now = 1_000_000_000;
        let exact = |ago| Published {
            at: now - ago,
            precision: Precision::Exact,
        };
        assert_eq!(published(&exact(10), now), "just now");
        assert_eq!(published(&exact(2 * 3600), now), "2 hours ago");
        assert_eq!(published(&exact(86_400), now), "1 day ago");
        assert_eq!(published(&exact(400 * 86_400), now), "1 year ago");
        let day = Published {
            at: now - 2 * 3600,
            precision: Precision::Day,
        };
        assert_eq!(published(&day, now), "today");
    }
}
