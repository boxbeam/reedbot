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
    #[error("Invalid weekday: {0}")]
    InvalidWeekday(String),
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

    delay: num=num unit=<('h'|'m'|'s'|'w'|'d')> -> u64 {
        let multiplier = match unit {
            "w" => 7 * 24 * 60 * 60 * 1000,
            "d" => 24 * 60 * 60 * 1000,
            "h" => 60 * 60 * 1000,
            "m" => 60 * 1000,
            "s" => 1000,
            _ => unreachable!(),
        };
        num * multiplier
    }

    months: num=num "mo" -> TimeModifier {
        TimeModifier::Months(num)
    }

    weekday: day=<
            "monday"
            | "Monday"
            | "tuesday"
            | "Tuesday"
            | "wednesday"
            | "Wednesday"
            | "thursday"
            | "Thursday"
            | "friday"
            | "Friday"
            | "saturday"
            | "Saturday"
            | "sunday"
            | "Sunday"
            >
        -> TimeModifier {
        TimeModifier::Weekday(match &*day.to_lowercase() {
            "monday" => Weekday::Monday,
            "tuesday" => Weekday::Tuesday,
            "wednesday" => Weekday::Wednesday,
            "thursday" => Weekday::Thursday,
            "friday" => Weekday::Friday,
            "saturday" => Weekday::Saturday,
            "sunday" => Weekday::Sunday,
            _ => return Err(ParseTimeError::InvalidWeekday(day.to_string()))
        }.to_monday_zero_offset())
    }

    delays: delays=delay+ -> TimeModifier { TimeModifier::Delay(delays.into_iter().sum()) }

    time_of_day: hour=num minute=(":" num)? ampm=<("am"|"pm")> -> TimeModifier {
        let minute = minute.unwrap_or(0);
        let pm = ampm == "pm";
        let mut hour = hour % 12;
        if pm {
            hour += 12;
        }
        TimeModifier::TimeOfDay { minute, hour }
    }

    date: year=num "-" month=num "-" day=num -> TimeModifier {
        TimeModifier::Date { year, month, day }
    }

    modifier = (months | delays | time_of_day | date | weekday) -> TimeModifier;

    schedule_reminder: ("r" | "remindme" | "reminder") " " time=time ";" " "? message=<.+> -> Command {
        Command::ScheduleReminder(time, message.to_string())
    }

    cancel_reminder: ("cancelreminder" | "cr") " " id=num -> Command { Command::CancelReminder(id) }

    set_interval: ("setinterval" | "si") " " id=num " " modifiers=modifier$" "+ -> Command {
        Command::SetInterval(id, modifiers)
    }

    clear_interval: ("clearinterval" | "ci") " " id=num -> Command { Command::ClearInterval(id) }

    list_reminders: ("reminders" | "rs") -> Command { Command::ListReminders }

    set_timezone: ("tz" | "timezone") " " timezone=<.+> -> Command {
        Command::SetTimezone(timezone.to_string())
    }

    help: ("h" | "help") -> Command { Command::Help }

    pub command = "$" (schedule_reminder | set_interval | clear_interval | list_reminders | help | cancel_reminder | set_timezone) -> Command;

    pub time: modifiers=modifier$" "+ -> Zoned {
        let mut date = Zoned::now().with_time_zone(__ctx.data().clone());
        for modifier in modifiers {
            date = modifier.modify(date).unwrap();
        }
        date
    }

}
