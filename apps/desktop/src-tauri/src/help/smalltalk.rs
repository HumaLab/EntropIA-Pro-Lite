//! Local small talk (S22, D19): greetings, thanks and date/time questions
//! answered deterministically, with no API call, no key and no sources.
//!
//! Accent-folded and punctuation-tolerant («Qué hora es???» counts). A
//! message that mixes small talk with anything else («hola, ¿cómo importo
//! un PDF?») is NOT small talk: every comma- or sentence-separated clause
//! must classify, or the whole message goes to the normal path. Word
//! boundaries matter («ahora» is not «hora», «importo» carries no time).

use super::clock::{date_en, date_es, time_en, time_es, LocalDateTime};
use super::text;

/// What kind of small talk a message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmallTalk {
    Greeting,
    Thanks,
    Time,
    Date,
}

/// The answer language: English when the small talk itself is English,
/// Spanish otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmallTalkLang {
    Spanish,
    English,
}

/// The local answer for `question`, or `None` when it must go to the normal
/// path (FAQ or agent). `now` is injected so tests are deterministic;
/// production passes [`super::clock::now_local`].
pub(crate) fn answer_small_talk(question: &str, now: &LocalDateTime) -> Option<String> {
    let (kind, lang) = detect_small_talk(question)?;
    Some(match (kind, lang) {
        (SmallTalk::Greeting, SmallTalkLang::Spanish) => {
            "¡Hola! ¿Qué te gustaría saber sobre EntropIA?".to_string()
        }
        (SmallTalk::Greeting, SmallTalkLang::English) => {
            "Hello! What would you like to know about EntropIA?".to_string()
        }
        (SmallTalk::Thanks, SmallTalkLang::Spanish) => {
            "De nada. Si tienes otra duda sobre EntropIA, aquí estoy.".to_string()
        }
        (SmallTalk::Thanks, SmallTalkLang::English) => {
            "You're welcome! If you have another question about EntropIA, I'm here.".to_string()
        }
        (SmallTalk::Time, SmallTalkLang::Spanish) => {
            format!("Son las {} del {}.", time_es(now), date_es(now))
        }
        (SmallTalk::Time, SmallTalkLang::English) => {
            format!("It's {} on {}.", time_en(now), date_en(now))
        }
        (SmallTalk::Date, SmallTalkLang::Spanish) => {
            format!("Hoy es {}.", date_es(now))
        }
        (SmallTalk::Date, SmallTalkLang::English) => {
            format!("Today is {}.", date_en(now))
        }
    })
}

/// Which small talk `question` is, with its language — or `None` when any
/// clause is not small talk. Accent-folded, case-insensitive,
/// punctuation-tolerant.
pub(crate) fn detect_small_talk(question: &str) -> Option<(SmallTalk, SmallTalkLang)> {
    let clauses = split_clauses(question);
    if clauses.is_empty() {
        return None;
    }
    let mut kinds = Vec::with_capacity(clauses.len());
    for clause in &clauses {
        kinds.push(classify_clause(clause)?);
    }
    // The most specific kind wins: a time question with a greeting («hola,
    // ¿qué hora es?») is answered with the time.
    kinds.into_iter().min_by_key(|(kind, _)| match kind {
        SmallTalk::Time => 0,
        SmallTalk::Date => 1,
        SmallTalk::Thanks => 2,
        SmallTalk::Greeting => 3,
    })
}

/// Splits a message into clauses on sentence ends, commas and line breaks —
/// never on colons, so «14:05» survives. Empty clauses are dropped.
fn split_clauses(question: &str) -> Vec<String> {
    question
        .split(|c: char| ['.', '!', '?', ';', ',', '\n', '\r', '…', '¿', '¡'].contains(&c))
        .map(normalize)
        .filter(|clause| !clause.is_empty())
        .collect()
}

/// Accent-folds a clause and collapses every non-alphanumeric run to one
/// space, so «Qué hora es???» meets «que hora es» and «Buen día» meets
/// «buen dia».
fn normalize(clause: &str) -> String {
    text::fold(clause)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Classifies one normalized clause, or `None` when it is not small talk.
fn classify_clause(clause: &str) -> Option<(SmallTalk, SmallTalkLang)> {
    if let Some(lang) = greeting_lang(clause) {
        return Some((SmallTalk::Greeting, lang));
    }
    if let Some(lang) = thanks_lang(clause) {
        return Some((SmallTalk::Thanks, lang));
    }
    time_kind(clause)
}

/// A whole-clause greeting, or `None`. Full matches only: «hola» never meets
/// «hola mundo».
fn greeting_lang(clause: &str) -> Option<SmallTalkLang> {
    match clause {
        "hola" | "buenas" | "buenos dias" | "buen dia" | "buenas tardes" | "buenas noches"
        | "que tal" | "como estas" | "como esta" | "como estan" | "como va" | "como van"
        | "como andas" | "saludos" => Some(SmallTalkLang::Spanish),
        "hello" | "hi" | "hey" | "good morning" | "good afternoon" | "good evening"
        | "good night" | "how are you" | "how is it going" | "how are you doing" => {
            Some(SmallTalkLang::English)
        }
        _ => None,
    }
}

/// A whole-clause thanks, or `None`. «gracias por tu ayuda» counts;
/// «gracias por importar» does not (it asks about importing).
fn thanks_lang(clause: &str) -> Option<SmallTalkLang> {
    match clause {
        "gracias"
        | "muchas gracias"
        | "mil gracias"
        | "muchisimas gracias"
        | "gracias por tu ayuda"
        | "gracias por su ayuda"
        | "gracias por la ayuda"
        | "muchas gracias por tu ayuda"
        | "muchas gracias por su ayuda"
        | "muchas gracias por la ayuda" => Some(SmallTalkLang::Spanish),
        "thanks"
        | "thank you"
        | "thanks a lot"
        | "thank you very much"
        | "thanks so much"
        | "thanks very much"
        | "many thanks" => Some(SmallTalkLang::English),
        _ => None,
    }
}

/// A time or date question clause, or `None`. Word-based, so «ahora» never
/// meets «hora» and «diario» never meets «dia»: «hora»/«time» ask the time,
/// «fecha»/«date» ask the date, and «dia»/«hoy» need a question word («hoy»
/// alone is not a question).
fn time_kind(clause: &str) -> Option<(SmallTalk, SmallTalkLang)> {
    let words: Vec<&str> = clause.split(' ').collect();
    let has = |word: &str| words.contains(&word);
    if has("hora") || has("horas") || has("time") {
        let lang = if has("time") {
            SmallTalkLang::English
        } else {
            SmallTalkLang::Spanish
        };
        return Some((SmallTalk::Time, lang));
    }
    if has("fecha") || has("fechas") || has("date") || has("dates") {
        let lang = if has("date") || has("dates") {
            SmallTalkLang::English
        } else {
            SmallTalkLang::Spanish
        };
        return Some((SmallTalk::Date, lang));
    }
    let day_word = has("dia") || has("dias") || has("day") || has("days");
    let today_word = has("hoy") || has("today");
    if day_word || today_word {
        let question_word = has("que")
            || has("cual")
            || has("cuales")
            || has("cuando")
            || has("what")
            || has("when")
            || has("which");
        if question_word {
            let lang = if has("day") || has("days") || has("today") || has("what") || has("when") {
                SmallTalkLang::English
            } else {
                SmallTalkLang::Spanish
            };
            return Some((SmallTalk::Date, lang));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::clock::Weekday;
    use super::*;

    fn fixed_now() -> LocalDateTime {
        LocalDateTime {
            year: 2026,
            month: 11,
            day: 14,
            weekday: Weekday::Saturday,
            hour: 14,
            minute: 5,
        }
    }

    #[test]
    fn owner_greetings_are_small_talk() {
        // L53: «Hola» got four unrelated manual sources; it never should.
        for greeting in [
            "Hola",
            "hola",
            "¡Hola!",
            "Buenas",
            "buenas tardes",
            "Buen día",
            "buenos dias",
            "buenas noches",
            "Hello",
            "hello",
            "Hi",
            "hi!",
            "Hey",
            "Saludos",
            "¿Qué tal?",
            "que tal",
            "¿Cómo estás?",
        ] {
            assert!(
                detect_small_talk(greeting).is_some_and(|(kind, _)| kind == SmallTalk::Greeting),
                "{greeting:?} must be a greeting"
            );
        }
    }

    #[test]
    fn owner_time_question_is_small_talk() {
        // L53: «Qué hora es???» got four unrelated manual sources and an
        // answer saying the clock cannot be read.
        for asked in [
            "Qué hora es???",
            "¿Qué hora es?",
            "que hora es",
            "¿Qué hora es ahora?",
            "me decís la hora",
            "dame la hora",
            "What time is it?",
            "what time is it",
            "what's the time?",
            "can you tell me the time?",
        ] {
            assert!(
                detect_small_talk(asked).is_some_and(|(kind, _)| kind == SmallTalk::Time),
                "{asked:?} must ask the time"
            );
        }
    }

    #[test]
    fn thanks_and_dates_are_small_talk() {
        for (asked, kind) in [
            ("Gracias", SmallTalk::Thanks),
            ("gracias", SmallTalk::Thanks),
            ("Muchas gracias", SmallTalk::Thanks),
            ("muchas gracias por tu ayuda", SmallTalk::Thanks),
            ("Mil gracias", SmallTalk::Thanks),
            ("Thanks", SmallTalk::Thanks),
            ("thanks a lot", SmallTalk::Thanks),
            ("Thank you very much", SmallTalk::Thanks),
            ("¿Qué día es hoy?", SmallTalk::Date),
            ("que dia es hoy", SmallTalk::Date),
            ("qué fecha es", SmallTalk::Date),
            ("¿Qué fecha es hoy?", SmallTalk::Date),
            ("What day is it?", SmallTalk::Date),
            ("what day is today", SmallTalk::Date),
            ("What's the date?", SmallTalk::Date),
            ("what is today's date", SmallTalk::Date),
        ] {
            assert_eq!(
                detect_small_talk(asked).map(|(kind, _)| kind),
                Some(kind),
                "{asked:?} must be {kind:?}"
            );
        }
    }

    #[test]
    fn mixed_messages_are_not_small_talk() {
        // L53/D19: mixed messages go to the normal path.
        for mixed in [
            "hola, ¿cómo importo un PDF?",
            "Hola, ¿cómo creo mi primera colección?",
            "gracias, ¿cómo hago OCR?",
            "Buenas, ¿dónde está el botón Guardar cambios?",
            "hello, how do I import a PDF?",
            "¿En qué me puedes ayudar?",
            "en que me puedes ayudar",
        ] {
            assert_eq!(
                detect_small_talk(mixed),
                None,
                "{mixed:?} must go to the normal path"
            );
        }
    }

    #[test]
    fn lookalikes_are_not_time_questions() {
        // Word boundaries: «ahora» is not «hora», «importo» carries no
        // time, «hoy» alone is not a question.
        for asked in [
            "¿y ahora qué?",
            "que hago ahora",
            "¿Cómo importo un PDF?",
            "hoy",
            "today",
            "¿Qué tiempo hace?",
            "el diario de hoy",
            "mejora la imagen",
        ] {
            assert_eq!(detect_small_talk(asked), None, "{asked:?} must not match");
        }
    }

    #[test]
    fn empty_and_noise_are_not_small_talk() {
        for asked in ["", "   ", "???", "!!!", ",", "123", "hola mundo"] {
            assert_eq!(detect_small_talk(asked), None, "{asked:?} must not match");
        }
    }

    #[test]
    fn time_answer_uses_the_injected_clock_in_spanish() {
        // S22: «Son las 14:05 del viernes 14 de noviembre de 2026.»
        let now = LocalDateTime {
            weekday: Weekday::Friday,
            ..fixed_now()
        };
        assert_eq!(
            answer_small_talk("Qué hora es???", &now).as_deref(),
            Some("Son las 14:05 del viernes 14 de noviembre de 2026.")
        );
    }

    #[test]
    fn time_and_date_answers_come_in_both_languages() {
        let now = fixed_now();
        assert_eq!(
            answer_small_talk("What time is it?", &now).as_deref(),
            Some("It's 2:05 PM on Saturday, November 14, 2026.")
        );
        assert_eq!(
            answer_small_talk("¿Qué día es hoy?", &now).as_deref(),
            Some("Hoy es sábado 14 de noviembre de 2026.")
        );
        assert_eq!(
            answer_small_talk("What day is it?", &now).as_deref(),
            Some("Today is Saturday, November 14, 2026.")
        );
    }

    #[test]
    fn greeting_and_thanks_invite_entropia_questions() {
        let now = fixed_now();
        let greeting = answer_small_talk("Hola", &now).expect("greeting answers locally");
        assert!(
            greeting.contains("EntropIA"),
            "the greeting invites an EntropIA question: {greeting}"
        );
        let thanks = answer_small_talk("gracias", &now).expect("thanks answers locally");
        assert!(
            thanks.starts_with("De nada"),
            "thanks starts with «De nada»: {thanks}"
        );
        let english = answer_small_talk("hello", &now).expect("english greeting answers locally");
        assert!(
            english.contains("EntropIA"),
            "the english greeting invites an EntropIA question: {english}"
        );
    }
}
