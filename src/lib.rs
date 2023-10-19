use chrono::{Duration, NaiveDate, NaiveTime};
use once_cell::sync::Lazy;
use scraper::{ElementRef, Html, Selector};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize, Serializer};

#[cfg(feature = "ics")]
use ics::{
    properties::{DtEnd, DtStart, Location, RRule, Summary, TzName},
    Daylight, Standard, TimeZone,
};

macro_rules! selector {
    ($name:ident, $query:expr) => {
        static $name: Lazy<Selector> = Lazy::new(|| Selector::parse($query).unwrap());
    };
}

selector!(TITLE, "title");
selector!(START_YEAR, "select[name=year] > option[selected]");
selector!(WEEK_NUMBER, "th.week_number");
selector!(WEEKS, "div.calendar > table.week_table > tbody");
selector!(START_DATE, "tr > td.week_header > nobr");
selector!(ROWS, "tr");
selector!(COLUMNS, "td");
selector!(RESOURCE, "span.resource");
selector!(ANCHOR, "a");

pub trait ToHTML {
    fn to_html(self) -> Html;
}

impl ToHTML for &str {
    fn to_html(self) -> Html {
        Html::parse_document(self)
    }
}

impl ToHTML for Html {
    fn to_html(self) -> Html {
        self
    }
}

#[cfg(feature = "serde")]
fn serialize_naive_time<S: Serializer>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error> {
    use chrono::Timelike;
    let formatted_time = format!("{:02}:{:02}", time.hour(), time.minute());
    serializer.serialize_str(&formatted_time)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
pub struct Calendar {
    pub name: String,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
pub struct Event {
    pub date: NaiveDate,
    #[serde(serialize_with = "serialize_naive_time")]
    pub start: NaiveTime,
    #[serde(serialize_with = "serialize_naive_time")]
    pub end: NaiveTime,
    pub title: String,
    pub location: Option<String>,
}

impl Calendar {
    pub fn from_html(html: impl ToHTML) -> Option<Self> {
        let html = html.to_html();

        let name = html.select(&TITLE).next()?.inner_html().trim().to_string();

        let mut start_year = html
            .select(&START_YEAR)
            .next()?
            .inner_html()
            .parse::<i32>()
            .ok()?;

        let mut events = Vec::new();

        for (idx, week) in html.select(&WEEKS).enumerate() {
            let week_number = week
                .select(&WEEK_NUMBER)
                .next()?
                .inner_html()
                .split(' ')
                .nth(1)?
                .parse::<usize>()
                .ok()?;

            if week_number == 1 && idx > 0 {
                start_year += 1;
            }

            let start_date_raw = week.select(&START_DATE).next()?.inner_html();

            let mut day_month = start_date_raw
                .split(' ')
                .nth(1)?
                .trim_end_matches('.')
                .split('.');

            let start_day = day_month.next()?.parse::<u32>().ok()?;
            let start_month = day_month.next()?.parse::<u32>().ok()?;

            for row in week.select(&ROWS).skip(1) {
                let monday = NaiveDate::from_ymd_opt(start_year, start_month, start_day)?;
                let mut day_index = 0;

                for column in row.select(&COLUMNS) {
                    let class = column.value().classes().next()?;

                    if class.starts_with("week_separatorcell") {
                        day_index += 1;
                    }

                    if class != "week_block" {
                        continue;
                    }

                    let date = monday + Duration::days(day_index);
                    events.push(Event::from_element(column, date)?);
                }
            }
        }

        Some(Calendar { events, name })
    }
}

impl Event {
    fn from_element(element: ElementRef, date: NaiveDate) -> Option<Event> {
        let details = element.select(&ANCHOR).next()?.inner_html();
        let mut details_split = details.split("<br>");

        let times_raw = details_split.next()?;
        let mut times_raw_split = times_raw.split("&nbsp;-");

        let start = NaiveTime::parse_from_str(times_raw_split.next()?, "%H:%M").ok()?;
        let end = NaiveTime::parse_from_str(times_raw_split.next()?, "%H:%M").ok()?;

        let title = details_split.next()?.replace("&amp;", "&");

        let location = element
            .select(&RESOURCE)
            .nth(1)
            .map(|location| location.inner_html());

        Some(Event {
            date,
            start,
            end,
            title,
            location,
        })
    }
}

#[cfg(feature = "ics")]
impl Calendar {
    pub fn to_ics<'a>(self) -> ics::ICalendar<'a> {
        let mut cet_standard = Standard::new("19701025T030000", "+0200", "+0100");
        cet_standard.push(TzName::new("CET"));
        cet_standard.push(RRule::new("FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU"));

        let mut cest_daylight = Daylight::new("19700329T020000", "+0100", "+0200");
        cest_daylight.push(TzName::new("CEST"));
        cest_daylight.push(RRule::new("FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU"));

        let mut timezone = TimeZone::daylight("Europe/Berlin", cest_daylight);
        timezone.add_standard(cet_standard);

        let mut icalendar = ics::ICalendar::new("2.0", self.name);
        icalendar.add_timezone(timezone);

        for event in self.events {
            icalendar.add_event(event.to_ics())
        }

        icalendar
    }
}

#[cfg(feature = "ics")]
impl Event {
    pub fn to_ics<'a>(self) -> ics::Event<'a> {
        let start = format!(
            "{}T{}00",
            self.date.format("%Y%m%d"),
            self.start.format("%H%M")
        );

        let end = format!(
            "{}T{}00",
            self.date.format("%Y%m%d"),
            self.end.format("%H%M")
        );

        let id = format!("{}_{}", start, self.title.replace(' ', "-"));

        let mut ics_event = ics::Event::new(id, start.clone());

        ics_event.push(DtStart::new(start));
        ics_event.push(DtEnd::new(end));
        ics_event.push(Summary::new(self.title));

        if let Some(location) = self.location {
            ics_event.push(Location::new(location));
        }

        ics_event
    }
}