//! The emoji panel's own list: each emoji with names to search it by (Russian and
//! English words, written for Nimbus), and the emoji of a server by name.

/// (emoji, words it is found by), the usual ones first.
pub const LIST: &[(&str, &str)] = &[
    ("😀", "улыбка радость рад smile grin happy"),
    ("😂", "смех слёзы смешно ржу laugh joy lol"),
    ("🤣", "катаюсь смех ору rofl"),
    ("😅", "пот неловко uff sweat"),
    ("😉", "подмигивает wink"),
    ("😍", "влюблён сердечки love eyes"),
    ("🥰", "любовь милота adore"),
    ("😎", "круто очки cool sunglasses"),
    ("🤔", "думаю размышляю think hmm"),
    ("🙄", "закатил глаза eyeroll"),
    ("😏", "ухмылка smirk"),
    ("😴", "сплю сон sleep zzz"),
    ("😭", "плачу рыдаю sob cry"),
    ("😢", "грустно слеза sad tear"),
    ("😱", "ужас страх scream shock"),
    ("😡", "злой гнев angry rage"),
    ("🤯", "взрыв мозга mindblown"),
    ("🥹", "трогательно тронут touched"),
    ("🫡", "салют честь salute"),
    ("🫶", "сердце руки heart hands"),
    ("😊", "приятно румянец blush"),
    ("😁", "ухмыляюсь зубы beam"),
    ("😇", "ангел нимб angel"),
    ("🙃", "вверх ногами upside"),
    ("😘", "поцелуй kiss"),
    ("😋", "вкусно yum"),
    ("😜", "язык шалю wink tongue"),
    ("🤗", "обнимаю hug"),
    ("🤫", "тихо ссс shush"),
    ("🤭", "хихи giggle"),
    ("🥳", "праздник вечеринка party"),
    ("😤", "пар фыркаю huff"),
    ("😬", "гримаса grimace"),
    ("🤢", "тошнит sick nausea"),
    ("🤮", "рвота vomit"),
    ("😷", "маска болею mask"),
    ("🥶", "холодно мёрзну cold freeze"),
    ("🥵", "жарко hot"),
    ("😳", "смущён shy flushed"),
    ("🥺", "умоляю просьба pleading"),
    ("😐", "ничего нейтрально neutral"),
    ("😑", "без эмоций expressionless"),
    ("😶", "без рта молчу silent"),
    ("😒", "недоволен unamused"),
    ("😞", "разочарован disappointed"),
    ("😔", "задумчиво pensive"),
    ("😖", "мучаюсь confounded"),
    ("😩", "устал weary"),
    ("💀", "череп умер skull dead"),
    ("👻", "призрак ghost"),
    ("🤡", "клоун clown"),
    ("🤖", "робот robot"),
    ("👽", "инопланетянин alien"),
    ("💩", "какашка poop"),
    ("👍", "класс лайк согласен thumbsup like yes"),
    ("👎", "дизлайк против thumbsdown dislike no"),
    ("👏", "аплодисменты хлопаю clap"),
    ("🙏", "пожалуйста спасибо молитва please thanks pray"),
    ("💪", "сила мышцы strong muscle"),
    ("👌", "окей ok okay"),
    ("✌️", "мир победа peace victory"),
    ("🤝", "рукопожатие договорились handshake deal"),
    ("🤞", "скрестил пальцы удача fingers luck"),
    ("👋", "привет пока махаю wave hello"),
    ("🤙", "позвони call"),
    ("👀", "смотрю глаза eyes look"),
    ("🧠", "мозг ум brain"),
    ("🫵", "ты указываю you point"),
    ("☝️", "один вверх one up"),
    ("🖕", "грубый жест"),
    ("🤌", "итальянский жест pinch"),
    ("🔥", "огонь круто fire hot lit"),
    ("❤️", "сердце любовь красное heart love"),
    ("🧡", "оранжевое сердце orange heart"),
    ("💛", "жёлтое сердце yellow heart"),
    ("💚", "зелёное сердце green heart"),
    ("💙", "синее сердце blue heart"),
    ("💜", "фиолетовое сердце purple heart"),
    ("🖤", "чёрное сердце black heart"),
    ("🤍", "белое сердце white heart"),
    ("💔", "разбитое сердце broken heart"),
    ("💯", "сто баллов идеально hundred perfect"),
    ("✨", "искры блеск sparkles"),
    ("🌟", "звезда star"),
    ("⭐", "звёздочка star"),
    ("🎉", "праздник ура салют tada party"),
    ("🎊", "конфетти confetti"),
    ("🎁", "подарок gift present"),
    ("✅", "готово галочка check done yes"),
    ("❌", "крестик нет cross no"),
    ("⚠️", "внимание предупреждение warning"),
    ("❓", "вопрос question"),
    ("❗", "восклицание exclamation"),
    ("⚡", "молния быстро lightning zap"),
    ("💥", "бум взрыв boom"),
    ("💤", "сон zzz sleep"),
    ("💬", "сообщение чат speech chat"),
    ("🔔", "колокольчик уведомление bell"),
    ("🔒", "замок закрыто lock"),
    ("🔑", "ключ key"),
    ("📌", "кнопка закреплено pin"),
    ("📎", "скрепка clip"),
    ("📷", "камера фото camera photo"),
    ("🎥", "видеокамера video camera"),
    ("🎮", "игра геймпад game controller"),
    ("🎧", "наушники музыка headphones music"),
    ("🎵", "нота музыка note music"),
    ("🎤", "микрофон mic microphone"),
    ("💻", "ноутбук компьютер laptop computer"),
    ("🖥️", "монитор экран monitor screen"),
    ("⌨️", "клавиатура keyboard"),
    ("🖱️", "мышь mouse"),
    ("📱", "телефон phone"),
    ("🕹️", "джойстик joystick"),
    ("🏆", "кубок победа trophy win"),
    ("⚽", "футбол мяч football soccer"),
    ("🏀", "баскетбол basketball"),
    ("🍕", "пицца pizza"),
    ("🍔", "бургер burger"),
    ("🍟", "картошка фри fries"),
    ("🌭", "хот-дог hotdog"),
    ("🍿", "попкорн popcorn"),
    ("🍩", "пончик donut"),
    ("🍰", "торт cake"),
    ("🍺", "пиво beer"),
    ("🍷", "вино wine"),
    ("☕", "кофе coffee"),
    ("🍵", "чай tea"),
    ("🥤", "напиток soda drink"),
    ("🍎", "яблоко apple"),
    ("🍌", "банан banana"),
    ("🍓", "клубника strawberry"),
    ("🐱", "кот кошка cat"),
    ("🐶", "собака пёс dog"),
    ("🦊", "лиса fox"),
    ("🐼", "панда panda"),
    ("🐸", "лягушка frog"),
    ("🐵", "обезьяна monkey"),
    ("🦄", "единорог unicorn"),
    ("🐧", "пингвин penguin"),
    ("🐢", "черепаха turtle"),
    ("🌈", "радуга rainbow"),
    ("☀️", "солнце sun"),
    ("🌙", "луна moon"),
    ("☁️", "облако cloud"),
    ("❄️", "снег холод snow"),
    ("🌊", "волна wave sea"),
    ("🚀", "ракета rocket"),
    ("🚗", "машина car"),
    ("✈️", "самолёт plane"),
    ("🏠", "дом home house"),
    ("💰", "деньги money"),
    ("💎", "алмаз gem diamond"),
    ("⏰", "будильник время alarm clock"),
];

/// How many emoji to show at most.
pub const SHOWN: usize = 48;

/// The emoji whose names have a word starting with the query (all, in order, for an empty one).
pub fn search(query: &str) -> Vec<&'static str> {
    let q = query.trim().to_lowercase();
    LIST.iter()
        .filter(|(_, names)| q.is_empty() || crate::mentions::matches(&q, &[names]))
        .map(|(e, _)| *e)
        .take(SHOWN)
        .collect()
}

/// A server's emoji: (id, name, animated), those whose name has a word starting with the query.
pub fn server_matches<'a>(all: &'a [(String, String, bool)], query: &str, limit: usize) -> Vec<&'a (String, String, bool)> {
    let q = query.trim().trim_matches(':').to_lowercase();
    all.iter()
        .filter(|(_, name, _)| q.is_empty() || crate::mentions::matches(&q, &[name]))
        .take(limit)
        .collect()
}

/// How a server emoji is written in a message: `<:name:id>` (`<a:name:id>` when animated).
pub fn markup(id: &str, name: &str, animated: bool) -> String {
    format!("<{}:{name}:{id}>", if animated { "a" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_by_russian_and_english_words() {
        assert!(search("кот").contains(&"🐱"));
        assert!(search("cat").contains(&"🐱"));
        assert!(search("СЕРД").contains(&"❤️"));
        assert!(search("heart").len() >= 8, "all the hearts");
        assert_eq!(search("такогоислова"), Vec::<&str>::new());
        assert_eq!(search("").len(), SHOWN);
    }

    #[test]
    fn every_emoji_has_a_name_and_none_repeats() {
        let mut seen = std::collections::HashSet::new();
        for (e, names) in LIST {
            assert!(!names.trim().is_empty(), "{e} has no names");
            assert!(seen.insert(*e), "{e} is listed twice");
        }
    }

    #[test]
    fn server_emoji_by_name_and_markup() {
        let all = vec![
            ("1".to_string(), "pog_champ".to_string(), false),
            ("2".to_string(), "dance".to_string(), true),
        ];
        assert_eq!(server_matches(&all, ":pog", 5).len(), 1);
        assert_eq!(server_matches(&all, "", 5).len(), 2);
        assert_eq!(markup("1", "pog_champ", false), "<:pog_champ:1>");
        assert_eq!(markup("2", "dance", true), "<a:dance:2>");
    }
}
