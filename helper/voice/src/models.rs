//! The wake words the engine carries: microWakeWord models (Apache-2.0, <https://github.com/esphome/micro-wake-word-models>, v2),
//! compiled in, so that nothing is read from a directory at run time and a model cannot be swapped under the engine.
//! `models/NOTICE.md` has their provenance.

use crate::engine::Word;
use wake_word::Model;
use wake_word::detector::Manifest;

/// A word the engine can listen for.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub id: &'static str,
    /// What is said.
    pub phrase: &'static str,
    model: &'static [u8],
    manifest: &'static str,
}

/// Every word. The first is the default: the owner's other voice assistants answer to "Hey Jarvis", so a second word that
/// is not that one keeps the TV from answering when they do.
pub const WORDS: [Info; 2] = [
    Info {
        id: "okay_nabu",
        phrase: "Okay Nabu",
        model: include_bytes!("../models/okay_nabu.tflite"),
        manifest: include_str!("../models/okay_nabu.json"),
    },
    Info {
        id: "hey_jarvis",
        phrase: "Hey Jarvis",
        model: include_bytes!("../models/hey_jarvis.tflite"),
        manifest: include_str!("../models/hey_jarvis.json"),
    },
];

/// The word with this id.
pub fn find(id: &str) -> Option<&'static Info> {
    WORDS.iter().find(|w| w.id == id)
}

/// How sure the model has to be, as the sliding-window average that triggers a wake. The middle is the cutoff the models are
/// published with; both ends are measured on the neighbouring Show device (Hey Jarvis: 0.99 gives 80 % recall and no false accepts
/// in an hour of speech; the published 0.97 gives 84 % and 0.19 an hour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    Strict,
    Normal,
    Lenient,
}

impl Sensitivity {
    pub fn cutoff(self) -> f32 {
        match self {
            Self::Strict => 0.99,
            Self::Normal => 0.97,
            Self::Lenient => 0.85,
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "strict" => Some(Self::Strict),
            "normal" => Some(Self::Normal),
            "lenient" => Some(Self::Lenient),
            _ => None,
        }
    }
}

/// The model of a word, with its manifest's cutoff set to `cutoff`.
pub fn load(id: &str, cutoff: f32) -> Result<Word, String> {
    let info = find(id).ok_or_else(|| format!("there is no wake word {id}"))?;
    let model = Model::parse(info.model.to_vec()).map_err(|e| format!("the model {id}: {e}"))?;
    let mut manifest =
        Manifest::parse(info.manifest).map_err(|e| format!("the manifest {id}: {e}"))?;
    manifest.probability_cutoff = cutoff;
    Ok(Word {
        id: id.to_owned(),
        model,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_loads_and_the_cutoff_is_the_one_asked_for() {
        for info in WORDS {
            let word = load(info.id, Sensitivity::Strict.cutoff()).unwrap();
            assert_eq!(word.manifest.probability_cutoff, 0.99);
            assert_eq!(word.manifest.wake_word, info.phrase);
        }
        assert!(load("alexa", 0.97).is_err());
    }

    #[test]
    fn sensitivity_names_map_to_three_cutoffs_in_order() {
        let s = |id| Sensitivity::parse(id).unwrap().cutoff();
        assert!(s("strict") > s("normal") && s("normal") > s("lenient"));
        assert!(Sensitivity::parse("loud").is_none());
    }
}
