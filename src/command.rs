use std::num::ParseIntError;

use jiff::{civil::Weekday, Zoned};
use thiserror::Error;
use untwine::prelude::*;

use crate::TimeModifier;

#[derive(Error, Debug)]
pub enum ParseTimeError {
    #[error("{0}")]
    Untwine(#[from] ParserError),
    #[error("{0}")]
    ParseInt(#[from] ParseIntError),
}

pub enum Command {
    ScheduleReminder(Zoned, String),
    CancelReminder(u64),
    SetInterval(u64, Vec<TimeModifier>),
    ClearInterval(u64),
    SetTimezone(String),
    ListReminders,
    Help,
}

parser! {
    [error = ParseTimeError, data = jiff::tz::TimeZone]
    num: num=<'0'-'9'+> -> u64 { num.parse()? }

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

    time_of_day: hour=num minute=(":" num)? specifier=<("am"|"pm"|"h")?> -> TimeModifier {
        let minute = minute.unwrap_or(0);
        let hour = match specifier {
            "am" => hour % 12,
            "pm" => (hour % 12) + 12,
            "h" | "" => hour % 24,
            _ => unreachable!("Unexpected time of day specifier")
        };
        TimeModifier::TimeOfDay { minute, hour }
    }

    date: year=num "-" month=num "-" day=num -> TimeModifier {
        TimeModifier::Date { year, month, day }
    }

    modifier = (months | delays | time_of_day | date | weekday_modifier) -> TimeModifier;

    match_commands = match {
        ("r" | "remindme" | "reminder") " " time=time ";" " "? message=<.+> => Command::ScheduleReminder(time, message.to_string()),
        ("h" | "help") => Command::Help,
        ("setinterval" | "si") " " id=num " " modifiers=modifier$" "+ => Command::SetInterval(id, modifiers),
        ("clearinterval" | "ci") " " id=num => Command::ClearInterval(id),
        ("cancelreminder" | "cr") " " id=num => Command::CancelReminder(id),
        ("reminders" | "rs") => Command::ListReminders,
        ("r" | "remindme" | "reminder") " " time=time ";" " "? message=<.+> => Command::ScheduleReminder(time, message.to_string()),
        ("tz" | "timezone") " " timezone=<.+> => Command::SetTimezone(timezone.to_string())
    } -> Command;

    pub command = "$" match_commands -> Command;

    pub time: modifiers=modifier$" "+ -> Zoned {
        let mut date = Zoned::now().with_time_zone(__ctx.data().clone());
        for modifier in modifiers {
            date = modifier.modify(date).unwrap();
        }
        date
    }

}
