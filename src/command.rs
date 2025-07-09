use std::num::ParseIntError;

use jiff::{civil::Weekday, Zoned};
use thiserror::Error;
use untwine::prelude::*;

use crate::{TimeFormat, TimeModifier};

#[derive(Error, Debug)]
pub enum ParseTimeError {
    #[error("{0}")]
    Untwine(#[from] ParserError),
    #[error("{0}")]
    ParseInt(#[from] ParseIntError),
}

pub enum Command {
    ScheduleReminder(Vec<Zoned>, String),
    CancelReminder(u64),
    SetInterval(u64, Vec<TimeModifier>),
    ClearInterval(u64),
    SetTimezone(String),
    SetTimeFormat(TimeFormat),
    ListReminders,
    Help,
}

pub enum Modifier {
    TimeModifier(TimeModifier),
    ModifierPermutations(Vec<TimeModifier>),
}

impl Modifier {
    pub fn into_time_modifiers(modifiers: Vec<Modifier>) -> Vec<Vec<TimeModifier>> {
        let mut final_modifiers = vec![vec![]];
        for modifier in modifiers {
            match modifier {
                Modifier::TimeModifier(time_modifier) => {
                    for modifiers in &mut final_modifiers {
                        modifiers.push(time_modifier.clone());
                    }
                }
                Modifier::ModifierPermutations(time_modifiers) => {
                    let all_variants = std::mem::take(&mut final_modifiers);
                    for permutation in time_modifiers {
                        let copied_modifiers = all_variants.iter().cloned().map(|mut v| {
                            v.push(permutation.clone());
                            v
                        });
                        final_modifiers.extend(copied_modifiers);
                    }
                }
            }
        }
        final_modifiers
    }
}

parser! {
    [error = ParseTimeError, data = jiff::tz::TimeZone]
    num: num=<'0'-'9'+> -> u64 { num.parse()? }
    comma = " "* "," " "*;

    unit = match {
        "w" => 7 * 24 * 60 * 60 * 1000,
        "d" => 24 * 60 * 60 * 1000,
        "h" => 60 * 60 * 1000,
        "m" => 60 * 1000,
        "s" => 1000,
    } -> u64;

    delay: num=num unit=unit -> u64 {
        num * unit
    }

    months: num=num "mo" -> TimeModifier {
        TimeModifier::Months(num)
    }

    weekday = match {
        ("monday" | "Monday") => Weekday::Monday,
        ("tuesday" | "Tuesday") => Weekday::Tuesday,
        ("wednesday" | "Wednesday") => Weekday::Wednesday,
        ("thursday" | "Thursday") => Weekday::Thursday,
        ("friday" | "Friday") => Weekday::Friday,
        ("saturday" | "Saturday") => Weekday::Saturday,
        ("sunday" | "Sunday") => Weekday::Sunday,
    } -> Weekday;

    weekday_modifier: weekday=weekday -> TimeModifier {
        TimeModifier::Weekday(weekday.to_monday_zero_offset())
    }

    delays: delays=delay+ -> TimeModifier { TimeModifier::Delay(delays.into_iter().sum()) }

    time_of_day: hour=num minute=(":" num)? specifier=<("am"|"pm")?> -> TimeModifier {
        let minute = minute.unwrap_or(0);
        let hour = match specifier {
            "am" => hour % 12,
            "pm" => (hour % 12) + 12,
            "" => hour % 24,
            _ => unreachable!("Unexpected time of day specifier")
        };
        TimeModifier::TimeOfDay { minute, hour }
    }

    date: year=num? "-" month=num? "-" day=num -> TimeModifier {
        let year = year.map(|year| year as i16);
        let month = month.map(|month| month as i8);
        let day = day as i8;
        TimeModifier::Date { year, month, day }
    }

    time_modifier = (months | delays | time_of_day | date | weekday_modifier) -> TimeModifier;

    modifier_permutations = "(" time_modifier$comma+ ")" -> Vec<TimeModifier>;

    modifier = match {
        modifier=time_modifier => Modifier::TimeModifier(modifier),
        permutations=modifier_permutations => Modifier::ModifierPermutations(permutations),
    } -> Modifier;

    time_format = match {
        "12h" => TimeFormat::H12,
        "24h" => TimeFormat::H24,
    } -> TimeFormat;

    match_commands = match {
        ("r" | "remindme" | "reminder") " " time=time ";" " "? message=<.+> => Command::ScheduleReminder(time, message.to_string()),
        ("h" | "help") => Command::Help,
        ("setinterval" | "si") " " id=num " " modifiers=time_modifier$" "+ => Command::SetInterval(id, modifiers),
        ("clearinterval" | "ci") " " id=num => Command::ClearInterval(id),
        ("cancelreminder" | "cr") " " id=num => Command::CancelReminder(id),
        ("reminders" | "rs") => Command::ListReminders,
        ("tz" | "timezone") " " timezone=<.+> => Command::SetTimezone(timezone.to_string()),
        ("tf" | "timeformat") " " time_format=time_format => Command::SetTimeFormat(time_format)
    } -> Command;

    pub command = "$" match_commands -> Command;

    pub time: modifiers=modifier$" "+ -> Vec<Zoned> {
        let modifier_permutations = Modifier::into_time_modifiers(modifiers);
        let date = Zoned::now().with_time_zone(__ctx.data().clone());

        let mut dates = vec![];
        for permutation in modifier_permutations {
            let mut date = date.clone();
            for modifier in permutation {
                date = modifier.modify(date).unwrap();
            }
            dates.push(date);
        }
        dates
    }

}
