use super::{group_counts, next_offset, Group};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use time::{Date, Duration, OffsetDateTime, UtcOffset};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    unit: Unit,
    pub(super) period: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Unit {
    Day,
    Week,
    Month,
}

const UNDATED: &str = "undated";

impl Selection {
    pub(super) fn overview(
        &self,
        sides: [Option<Group<'_>>; 2],
        offset: usize,
        limit: usize,
    ) -> Value {
        // The Boolean keeps undated evidence after the calendar periods.
        let mut periods = BTreeMap::<(bool, Option<Date>), [Option<Group<'_>>; 2]>::new();
        let mut scale: f64 = 0.0;
        for (index, side) in sides.into_iter().enumerate() {
            for item in side.into_iter().flat_map(|group| group.occurrences) {
                scale = scale.max(item.effect.abs());
                let start = self.start(item.time);
                periods.entry((start.is_none(), start)).or_default()[index]
                    .get_or_insert_with(Group::default)
                    .add(item);
            }
        }
        let total = periods.len();
        let rows: Vec<_> = periods
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|((_, start), sides)| {
                json!({
                    "period": start.map_or_else(|| UNDATED.to_string(), |date| date.to_string()),
                    "scope_a": sides[0].as_ref().map(group_counts),
                    "scope_b": sides[1].as_ref().map(group_counts),
                })
            })
            .collect();
        json!({
            "kind": "visibility_pattern_timeline", "unit": self.unit,
            "maximum_absolute_effect": scale,
            "total_periods": total, "rows": rows,
            "next_offset": next_offset(offset, rows.len(), total),
        })
    }

    pub(super) fn filter<'a>(&self, group: Group<'a>, period: &str) -> Option<Group<'a>> {
        let mut selected = Group::default();
        for item in group.occurrences {
            if self
                .start(item.time)
                .map_or_else(|| period == UNDATED, |date| date.to_string() == period)
            {
                selected.add(item);
            }
        }
        (!selected.occurrences.is_empty()).then_some(selected)
    }

    fn start(&self, time: Option<OffsetDateTime>) -> Option<Date> {
        let date = time?.to_offset(UtcOffset::UTC).date();
        let days = match self.unit {
            Unit::Day => 0,
            Unit::Week => date.weekday().number_days_from_monday(),
            Unit::Month => date.day() - 1,
        };
        Some(date.saturating_sub(Duration::days(i64::from(days))))
    }
}
