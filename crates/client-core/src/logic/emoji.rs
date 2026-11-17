//! In-app emoji picker content (X-08). Android and desktop also accept the OS emoji input.

pub const REACTIONS: &[&str] = &["\u{1F44D}", "\u{2764}\u{FE0F}", "\u{1F602}", "\u{1F389}", "\u{1F62E}", "\u{1F622}", "\u{1F64F}", "\u{1F680}"];

pub const GROUPS: &[(&str, &str)] = &[
    ("Smileys", "\u{1F600} \u{1F603} \u{1F604} \u{1F601} \u{1F606} \u{1F605} \u{1F602} \u{1F923} \u{1F642} \u{1F609} \u{1F60A} \u{1F607} \u{1F60D} \u{1F618} \u{1F60B} \u{1F61C} \u{1F914} \u{1F610} \u{1F644} \u{1F62E} \u{1F622} \u{1F62D} \u{1F620} \u{1F631} \u{1F634} \u{1F973}"),
    ("People", "\u{1F44D} \u{1F44E} \u{1F44F} \u{1F64C} \u{1F64F} \u{1F4AA} \u{1F44B} \u{270C}\u{FE0F} \u{1F44C} \u{1F91D} \u{1F440} \u{1F9E0}"),
    ("Symbols", "\u{2764}\u{FE0F} \u{1F494} \u{1F525} \u{2728} \u{1F389} \u{1F38A} \u{2705} \u{274C} \u{26A0}\u{FE0F} \u{1F4A1} \u{1F4AF} \u{1F680} \u{2B50} \u{1F3C6}"),
    ("Objects", "\u{1F4CE} \u{1F4C5} \u{1F4DD} \u{1F4E3} \u{1F514} \u{1F512} \u{1F527} \u{1F4BB} \u{1F4F1} \u{2615} \u{1F355} \u{1F382}"),
];

/// All picker emoji as one flat list.
pub fn all() -> Vec<String> {
    GROUPS.iter().flat_map(|(_, g)| g.split(' ').map(str::to_string)).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn picker_has_no_empty_entries() {
        let a = super::all();
        assert!(a.len() > 60 && a.iter().all(|e| !e.is_empty()));
    }
}
