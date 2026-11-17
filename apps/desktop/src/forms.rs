//! Bot forms (A-03, WF-01): state of the open modal and conversion to Slint models. No UI handles here.

use crate::{FormButtonItem, FormFieldItem, FormOptionItem};
use proto::automation::{Form, FormField, FormFieldKind};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    Message(String),
    Home { bot: String, form_id: String },
}

#[derive(Clone, Debug)]
pub struct FormState {
    pub src: Src,
    /// The pressed button when the form is a button's modal.
    pub button: Option<String>,
    pub form: Form,
    pub values: BTreeMap<String, Value>,
}

fn kind_num(k: FormFieldKind) -> i32 {
    match k {
        FormFieldKind::Text => 0,
        FormFieldKind::Multiline => 1,
        FormFieldKind::Boolean => 2,
        FormFieldKind::Select => 3,
        FormFieldKind::MultiSelect => 4,
    }
}

impl FormState {
    pub fn new(src: Src, button: Option<String>, form: Form) -> FormState {
        let mut values = BTreeMap::new();
        for f in &form.fields {
            if let Some(d) = &f.default {
                values.insert(f.id.clone(), d.clone());
            }
        }
        FormState { src, button, form, values }
    }

    fn text_of(&self, f: &FormField) -> String {
        self.values.get(&f.id).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    pub fn items(&self) -> Vec<FormFieldItem> {
        self.form
            .fields
            .iter()
            .map(|f| {
                let cur = self.values.get(&f.id);
                let options = f
                    .options
                    .iter()
                    .map(|o| FormOptionItem {
                        value: o.value.clone().into(),
                        label: if o.label.is_empty() { o.value.clone() } else { o.label.clone() }.into(),
                        on: match (f.kind, cur) {
                            (FormFieldKind::Select, Some(Value::String(s))) => *s == o.value,
                            (FormFieldKind::MultiSelect, Some(Value::Array(a))) => a.iter().any(|x| x.as_str() == Some(o.value.as_str())),
                            _ => false,
                        },
                    })
                    .collect::<Vec<_>>();
                FormFieldItem {
                    id: f.id.clone().into(),
                    kind: kind_num(f.kind),
                    label: if f.label.is_empty() { f.id.clone() } else { f.label.clone() }.into(),
                    placeholder: f.placeholder.clone().unwrap_or_default().into(),
                    required: f.required,
                    text: self.text_of(f).into(),
                    flag: cur.and_then(|v| v.as_bool()).unwrap_or(false),
                    options: crate::model::model(options),
                }
            })
            .collect()
    }

    pub fn set_text(&mut self, idx: usize, t: &str) {
        if let Some(f) = self.form.fields.get(idx) {
            self.values.insert(f.id.clone(), Value::String(t.to_string()));
        }
    }

    /// A boolean flips; a select picks the option (again clears nothing); a multi-select flips the option.
    pub fn toggle(&mut self, idx: usize, option: &str) {
        let Some(f) = self.form.fields.get(idx).cloned() else { return };
        match f.kind {
            FormFieldKind::Boolean => {
                let now = self.values.get(&f.id).and_then(|v| v.as_bool()).unwrap_or(false);
                self.values.insert(f.id, Value::Bool(!now));
            }
            FormFieldKind::Select => {
                self.values.insert(f.id, Value::String(option.to_string()));
            }
            FormFieldKind::MultiSelect => {
                let mut cur: Vec<Value> = self.values.get(&f.id).and_then(|v| v.as_array().cloned()).unwrap_or_default();
                if let Some(p) = cur.iter().position(|x| x.as_str() == Some(option)) {
                    cur.remove(p);
                } else {
                    cur.push(Value::String(option.to_string()));
                }
                self.values.insert(f.id, Value::Array(cur));
            }
            _ => {}
        }
    }

    /// Values to send: empty text and empty selections are left out so optional fields stay optional;
    /// booleans are always sent.
    pub fn submit_values(&self) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        for f in &self.form.fields {
            match (f.kind, self.values.get(&f.id)) {
                (FormFieldKind::Boolean, v) => {
                    out.insert(f.id.clone(), Value::Bool(v.and_then(|x| x.as_bool()).unwrap_or(false)));
                }
                (_, Some(Value::String(s))) if !s.trim().is_empty() => {
                    out.insert(f.id.clone(), Value::String(s.clone()));
                }
                (_, Some(Value::Array(a))) if !a.is_empty() => {
                    out.insert(f.id.clone(), Value::Array(a.clone()));
                }
                _ => {}
            }
        }
        out
    }
}

pub fn buttons(form: &Form) -> Vec<FormButtonItem> {
    form.buttons.iter().map(|b| FormButtonItem { id: b.id.clone().into(), label: b.label.clone().into(), style: b.style.clone().unwrap_or_default().into(), modal: b.form.is_some() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::automation::FormOption;

    fn form() -> Form {
        let opt = |v: &str| FormOption { value: v.into(), label: v.to_uppercase() };
        let field = |id: &str, kind, options: Vec<FormOption>| FormField { id: id.into(), kind, label: id.into(), placeholder: None, required: false, default: None, options };
        Form {
            id: "f".into(),
            title: Some("T".into()),
            description: None,
            fields: vec![
                field("note", FormFieldKind::Text, vec![]),
                field("urgent", FormFieldKind::Boolean, vec![]),
                field("cat", FormFieldKind::Select, vec![opt("a"), opt("b")]),
                field("tags", FormFieldKind::MultiSelect, vec![opt("x"), opt("y")]),
            ],
            buttons: vec![],
            submit_label: None,
        }
    }

    #[test]
    fn values_follow_the_inputs() {
        let mut s = FormState::new(Src::Message("m".into()), None, form());
        assert_eq!(s.submit_values().len(), 1, "only the boolean when nothing is filled in");
        s.set_text(0, "hello");
        s.toggle(1, "");
        s.toggle(2, "b");
        s.toggle(3, "x");
        s.toggle(3, "y");
        s.toggle(3, "x");
        let v = s.submit_values();
        assert_eq!(v["note"], "hello");
        assert_eq!(v["urgent"], true);
        assert_eq!(v["cat"], "b");
        assert_eq!(v["tags"], serde_json::json!(["y"]));
        let items = s.items();
        assert!(items[1].flag);
    }
}
