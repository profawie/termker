//! Which rows of a list are selected, by name, with the mouse and keyboard
//! rules people know from file managers: click picks one, Ctrl (Cmd on a
//! Mac) adds or removes one, Shift picks everything between, the arrow keys
//! move.

use std::collections::BTreeSet;

#[derive(Default, Debug)]
pub struct Selection {
    selected: BTreeSet<String>,
    /// Where a Shift range starts.
    anchor: Option<String>,
    /// The row the keyboard is on.
    cursor: Option<String>,
}

impl Selection {
    pub fn is_selected(&self, name: &str) -> bool {
        self.selected.contains(name)
    }

    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Plain click: only this row.
    pub fn select_only(&mut self, name: &str) {
        self.selected.clear();
        self.selected.insert(name.to_string());
        self.anchor = Some(name.to_string());
        self.cursor = Some(name.to_string());
    }

    /// Ctrl or Cmd click: add or remove this row.
    pub fn toggle(&mut self, name: &str) {
        if !self.selected.remove(name) {
            self.selected.insert(name.to_string());
        }
        self.anchor = Some(name.to_string());
        self.cursor = Some(name.to_string());
    }

    /// Shift click: every row from the anchor to this one. With `keep`
    /// (Ctrl+Shift) the rows already selected stay selected.
    pub fn extend_to(&mut self, names: &[&str], name: &str, keep: bool) {
        let Some(to) = names.iter().position(|other| *other == name) else {
            return;
        };
        let from = self
            .anchor
            .as_deref()
            .and_then(|anchor| names.iter().position(|other| *other == anchor))
            .unwrap_or(to);
        if !keep {
            self.selected.clear();
        }
        let (low, high) = (from.min(to), from.max(to));
        self.selected
            .extend(names[low..=high].iter().map(|name| name.to_string()));
        if self.anchor.is_none() {
            self.anchor = Some(name.to_string());
        }
        self.cursor = Some(name.to_string());
    }

    pub fn select_all(&mut self, names: &[&str]) {
        self.selected = names.iter().map(|name| name.to_string()).collect();
        if self.cursor.is_none() {
            self.cursor = names.first().map(|name| name.to_string());
            self.anchor = self.cursor.clone();
        }
    }

    /// Arrow keys: move `delta` rows from the cursor, staying in the list.
    /// With no cursor yet, down starts at the top and up at the bottom.
    pub fn step(&mut self, names: &[&str], delta: isize, extend: bool) {
        if names.is_empty() {
            return;
        }
        let last = names.len() as isize - 1;
        let target = match self.cursor_index(names) {
            Some(index) => (index as isize + delta).clamp(0, last),
            None if delta < 0 => last,
            None => 0,
        };
        self.jump(names, target as usize, extend);
    }

    /// Put the cursor on row `index`; Home and End, or a typed name.
    pub fn jump(&mut self, names: &[&str], index: usize, extend: bool) {
        let Some(name) = names.get(index) else {
            return;
        };
        if extend {
            self.extend_to(names, name, false);
        } else {
            self.select_only(name);
        }
    }

    pub fn cursor_index(&self, names: &[&str]) -> Option<usize> {
        let cursor = self.cursor.as_deref()?;
        names.iter().position(|name| *name == cursor)
    }

    /// The selected names in list order.
    #[cfg(test)]
    pub fn in_order<'a>(&self, names: &[&'a str]) -> Vec<&'a str> {
        names
            .iter()
            .copied()
            .filter(|name| self.selected.contains(*name))
            .collect()
    }

    /// Forget rows that are no longer in the list.
    pub fn retain(&mut self, names: &[&str]) {
        self.selected.retain(|name| names.contains(&name.as_str()));
        for kept in [&mut self.anchor, &mut self.cursor] {
            if kept.as_deref().is_some_and(|name| !names.contains(&name)) {
                *kept = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 5] = ["a", "b", "c", "d", "e"];

    fn picked(selection: &Selection) -> Vec<&str> {
        selection.in_order(&NAMES)
    }

    #[test]
    fn click_ctrl_click_and_shift_click() {
        let mut selection = Selection::default();
        selection.select_only("b");
        assert_eq!(picked(&selection), ["b"]);
        selection.toggle("d");
        assert_eq!(picked(&selection), ["b", "d"]);
        selection.toggle("b");
        assert_eq!(picked(&selection), ["d"]);
        // The range starts where the last click was, b.
        selection.extend_to(&NAMES, "a", false);
        assert_eq!(picked(&selection), ["a", "b"]);
        // A new range from the same anchor replaces the old one.
        selection.extend_to(&NAMES, "e", false);
        assert_eq!(picked(&selection), ["b", "c", "d", "e"]);
        selection.select_only("a");
        selection.extend_to(&NAMES, "b", true);
        selection.toggle("e");
        selection.extend_to(&NAMES, "d", true);
        assert_eq!(picked(&selection), ["a", "b", "d", "e"]);
    }

    #[test]
    fn arrow_keys() {
        let mut selection = Selection::default();
        selection.step(&NAMES, 1, false);
        assert_eq!(picked(&selection), ["a"]);
        selection.step(&NAMES, -1, false);
        assert_eq!(picked(&selection), ["a"]);
        selection.step(&NAMES, 1, true);
        selection.step(&NAMES, 1, true);
        assert_eq!(picked(&selection), ["a", "b", "c"]);
        assert_eq!(selection.cursor(), Some("c"));
        selection.step(&NAMES, -1, true);
        assert_eq!(picked(&selection), ["a", "b"]);
        selection.step(&NAMES, 10, false);
        assert_eq!(picked(&selection), ["e"]);

        let mut fresh = Selection::default();
        fresh.step(&NAMES, -1, false);
        assert_eq!(picked(&fresh), ["e"]);
    }

    #[test]
    fn select_all_and_reload() {
        let mut selection = Selection::default();
        selection.select_all(&NAMES);
        assert_eq!(picked(&selection), NAMES);
        assert_eq!(selection.cursor(), Some("a"));
        selection.retain(&["b", "c"]);
        assert_eq!(picked(&selection), ["b", "c"]);
        assert_eq!(selection.cursor(), None);
        selection.clear();
        assert!(selection.is_empty());
    }
}
