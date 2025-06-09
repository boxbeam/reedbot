use command::Command;
use jiff::{civil::Weekday, tz::TimeZone, Span, Zoned};
use serde::{Deserialize, Serialize};
use serenity::{
    all::{Context, CreateMessage, EventHandler, GatewayIntents, Http, Message, UserId},
    async_trait, Client,
};
use std::{collections::HashMap, fmt::Display, sync::LazyLock, time::Duration};
use thiserror::Error;
use tokio::sync::Mutex;
use untwine::prelude::ParserContext;

mod command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum TimeModifier {
    Delay(u64),
    Weekday(i8),
    TimeOfDay { hour: u64, minute: u64 },
    Date { year: u64, month: u64, day: u64 },
    Months(u64),
}

impl TimeModifier {
    fn modify(&self, mut datetime: Zoned) -> Result<Zoned, jiff::Error> {
        datetime.weekday();

        match self {
            TimeModifier::Delay(ms) => Ok(&datetime + Duration::from_millis(*ms as u64)),
            TimeModifier::TimeOfDay { hour, minute } => datetime
                .date()
                .at(*hour as i8, *minute as i8, 0, 0)
                .to_zoned(datetime.time_zone().clone()),
            TimeModifier::Date { year, month, day } => {
                jiff::civil::date(*year as i16, *month as i8, *day as i8)
                    .at(datetime.hour(), datetime.minute(), datetime.second(), 0)
                    .to_zoned(datetime.time_zone().clone())
            }
            TimeModifier::Weekday(weekday) => {
                datetime.nth_weekday(1, Weekday::from_monday_zero_offset(*weekday)?)
            }
            TimeModifier::Months(months) => {
                datetime += Span::new().months(*months as i64);
                Ok(datetime)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Reminder {
    time: Zoned,
    message: String,
    interval: Option<Vec<TimeModifier>>,
}

type ReminderCache = Mutex<HashMap<UserId, Vec<Reminder>>>;
static REMINDERS: LazyLock<ReminderCache> = LazyLock::new(Default::default);
static TIMEZONES: LazyLock<Mutex<HashMap<UserId, String>>> = LazyLock::new(Default::default);

#[derive(Error, Debug)]
enum CommandError {
    #[error("Invalid reminder ID: {0}")]
    InvalidID(u64),
    #[error("Time parsing error: {0}")]
    Jiff(#[from] jiff::Error),
}

async fn handle_command(user: UserId, command: Command) -> Result<String, CommandError> {
    let mut cache = REMINDERS.lock().await;
    use CommandError::*;
    match command {
        Command::ScheduleReminder(time, message) => {
            let list = cache.entry(user).or_default();
            let reminder = Reminder {
                time: time.clone(),
                message: message.clone(),
                interval: None,
            };
            list.push(reminder);
            list.sort_by(|a, b| a.time.cmp(&b.time));
            let id = list
                .iter()
                .enumerate()
                .find(|(_, e)| e.time == time && e.message == message)
                .map(|(i, _)| i)
                .expect("Reminder was not inserted");
            save();
            Ok(format!(
                "Scheduled reminder for {} (#{id})",
                format_time(&time)
            ))
        }
        Command::CancelReminder(id) => {
            let list = cache.get_mut(&user);
            if let Some(list) = list.filter(|l| l.len() > id as usize) {
                let reminder = list.remove(id as usize);
                save();
                Ok(format!("Removed reminder '{}'", reminder.message))
            } else {
                Err(InvalidID(id))
            }
        }
        Command::SetInterval(id, time_modifiers) => {
            let list = cache.get_mut(&user).ok_or(InvalidID(id))?;
            let reminder = list.get_mut(id as usize).ok_or(InvalidID(id))?;
            reminder.interval = Some(time_modifiers);
            save();
            Ok(format!(
                "Set interval for reminder '{}' (#{id})",
                &reminder.message
            ))
        }
        Command::ClearInterval(id) => {
            let list = cache.get_mut(&user).ok_or(InvalidID(id))?;
            let reminder = list.get_mut(id as usize).ok_or(InvalidID(id))?;
            reminder.interval = None;
            save();
            Ok(format!(
                "Cleared interval for reminder '{}' (#{id})",
                &reminder.message
            ))
        }
        Command::ListReminders => {
            let mut lines = vec![];
            for (id, reminder) in cache.get(&user).into_iter().flatten().enumerate() {
                let mut line = format!(
                    "{id}: {} - {}",
                    format_time(&reminder.time),
                    &reminder.message
                );
                if let Some(interval) = &reminder.interval {
                    let mut end = reminder.time.clone();
                    for modifier in interval {
                        end = modifier.modify(end)?;
                    }
                    let end = format_time(&end);
                    line.push_str(" (Repeats at ");
                    line.push_str(&end);
                    line.push_str(")");
                }
                lines.push(line);
            }

            if lines.is_empty() {
                return Ok("No reminders".into());
            }
            Ok(lines.join("\n"))
        }
        Command::SetTimezone(timezone) => {
            let mut timezones = TIMEZONES.lock().await;
            timezones.insert(user, timezone);
            save();
            Ok("Timezone set".into())
        }
        Command::Help => Ok([
            "Time modifier examples:",
            "1d - 1 day from now",
            "1w1h5m3s - 1 week, 1 hour, 5 minutes, 1 second from now",
            "3pm - 3:00 PM",
            "3:30pm - 3:30 PM",
            "2001-03-06 - March 6th, 2001",
            "1mo - 1 month",
            "tuesday - Tuesday",
            "1w tuesday - The next Tuesday in 1 week",
            "",
            "Commands:",
            "`$r|remindme|reminder <modifiers>; message` - Schedule a reminder",
            "`$cr <id>` - Cancel a reminder",
            "`$rs|reminders` - List reminders",
            "`$si|setinterval <id> <modifiers>` - Set a reminder to be repeated on an interval",
            "`$ci|clearinterval <id>` - Clear the interval of a reminder",
            "`$h|help` - Show help",
            "`$tz|timezone <timezone> - Set your timezone`",
        ]
        .join("\n")),
    }
}

const SAVE_FILE: &str = "reminders.json";
const TIMEZONE_FILE: &str = "timezones.json";

#[derive(Serialize, Deserialize)]
struct UserReminder {
    user: UserId,
    reminder: Reminder,
}

async fn load() {
    if !tokio::fs::try_exists(SAVE_FILE).await.unwrap() {
        return;
    }
    let contents = tokio::fs::read_to_string(SAVE_FILE).await.unwrap();
    let reminders: Vec<UserReminder> = serde_json::from_str(&contents).unwrap();
    let mut cache = REMINDERS.lock().await;
    cache.clear();

    for reminder in reminders {
        cache
            .entry(reminder.user)
            .or_default()
            .push(reminder.reminder);
    }

    for (_, list) in cache.iter_mut() {
        list.sort_by(|a, b| a.time.cmp(&b.time));
    }

    let Ok(timezones_json) = tokio::fs::read_to_string(TIMEZONE_FILE).await else {
        return;
    };
    let timezones = serde_json::from_str(&timezones_json).unwrap();
    *TIMEZONES.lock().await = timezones;
}

fn save() {
    tokio::spawn(async {
        let cache = REMINDERS.lock().await;

        let mut all_reminders = vec![];

        for (&user, reminders) in cache.iter() {
            all_reminders.extend(reminders.iter().map(|r| UserReminder {
                user,
                reminder: r.clone(),
            }));
        }

        let reminders_json = serde_json::to_string(&all_reminders).unwrap();
        tokio::fs::write(SAVE_FILE, reminders_json).await.unwrap();

        let timezones_json = serde_json::to_string(&*TIMEZONES.lock().await).unwrap();
        tokio::fs::write(TIMEZONE_FILE, timezones_json)
            .await
            .unwrap();
    });
}

fn log_error<T>(result: Result<T, impl Display>) {
    if let Err(err) = result {
        eprintln!("Failed to send reminder message: {err}");
    }
}

async fn reschedule(list: &mut Vec<Reminder>, reminder: &Reminder) {
    let Some(interval) = &reminder.interval else {
        return;
    };

    let mut time = reminder.time.clone();
    for modifier in interval {
        let Ok(modified) = modifier.modify(time) else {
            eprintln!("Failed to reschedule reminder {}", &reminder.message);
            return;
        };
        time = modified;
    }

    list.push(Reminder {
        time,
        message: reminder.message.clone(),
        interval: reminder.interval.clone(),
    });
    list.sort_by(|a, b| a.time.cmp(&b.time));
}

async fn process_reminders(http: &Http) {
    let mut cache = REMINDERS.lock().await;
    let now = Zoned::now();
    for (user, reminders) in cache.iter_mut() {
        while reminders.first().is_some_and(|f| f.time < now) {
            let first = reminders.remove(0);
            reschedule(reminders, &first).await;
            let message = format!("Reminder: {}", &first.message);
            log_error(user.dm(&http, CreateMessage::new().content(&message)).await);
        }
    }
    drop(cache);
    save();
}

fn format_time(time: &Zoned) -> String {
    time.strftime("%A, %B %d, %Y at %-I:%M%P %Z").to_string()
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let timezones = TIMEZONES.lock().await;
        let timezone = timezones
            .get(&msg.author.id)
            .map(|s| &**s)
            .unwrap_or("America/New_York");
        let timezone = jiff::tz::db().get(timezone).unwrap_or(TimeZone::system());
        drop(timezones);

        let mut parser_context = ParserContext::new(&msg.content, timezone);
        let result = parser_context.result(command::command(&parser_context));

        let command = match result {
            Ok(cmd) => cmd,
            Err(e) => {
                log_error(
                    msg.channel_id
                        .say(
                            &ctx.http,
                            format!("Invalid command: {e}", e = e.first().unwrap().1),
                        )
                        .await,
                );
                return;
            }
        };

        let response = match handle_command(msg.author.id, command).await {
            Ok(msg) => msg,
            Err(e) => format!("{e}"),
        };

        log_error(msg.channel_id.say(&ctx.http, response).await);
    }
}

#[tokio::main]
async fn main() {
    load().await;
    let token = std::env::var("DISCORD_TOKEN")
        .expect("Discord token not set in DISCORD_TOKEN environment variable");
    let intents = GatewayIntents::DIRECT_MESSAGES;
    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .await
        .unwrap();

    let http = client.http.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            process_reminders(&*http).await;
        }
    });

    client.start().await.unwrap();
}
