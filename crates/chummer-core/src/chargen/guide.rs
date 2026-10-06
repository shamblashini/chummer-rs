//! Guided creation: the order in which to fill in a new character, one
//! part at a time, for each build method.
//!
//! The order follows the core rulebook's "Creating a shadowrunner" steps
//! (SR5 p. 62ff) for the priority system and Run Faster's construction
//! kits (p. 62ff) for Sum-to-Ten, Point Buy and Life Modules. Priorities,
//! metatype and the magic or resonance priority are chosen in the New
//! Character wizard, so the first step only reviews them. Each step lists
//! the [`Area`]s whose [issues](super::issues) it shows; a step with
//! errors cannot be left with Next (warnings are fine).

use crate::character::Character;

use super::issues::{Area, Issue, IssueTab, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// Concept, priorities and metatype (chosen in the wizard).
    Concept,
    LifeModules,
    Attributes,
    SpecialAttributes,
    Qualities,
    ActiveSkills,
    KnowledgeSkills,
    Spells,
    AdeptPowers,
    ComplexForms,
    Cyberware,
    Gear,
    Vehicles,
    Contacts,
    CharacterInfo,
    Review,
}

impl Step {
    /// Stable name, e.g. for remembering the step per file.
    pub fn id(self) -> &'static str {
        match self {
            Step::Concept => "concept",
            Step::LifeModules => "lifemodules",
            Step::Attributes => "attributes",
            Step::SpecialAttributes => "special",
            Step::Qualities => "qualities",
            Step::ActiveSkills => "skills",
            Step::KnowledgeSkills => "knowledge",
            Step::Spells => "spells",
            Step::AdeptPowers => "powers",
            Step::ComplexForms => "complexforms",
            Step::Cyberware => "cyberware",
            Step::Gear => "gear",
            Step::Vehicles => "vehicles",
            Step::Contacts => "contacts",
            Step::CharacterInfo => "info",
            Step::Review => "review",
        }
    }

    pub fn parse(s: &str) -> Option<Step> {
        ALL.iter().copied().find(|st| st.id() == s.trim())
    }

    /// Short English title (goes through `lang.tr`).
    pub fn title(self) -> &'static str {
        match self {
            Step::Concept => "Concept & Metatype",
            Step::LifeModules => "Life Modules",
            Step::Attributes => "Attributes",
            Step::SpecialAttributes => "Special Attributes",
            Step::Qualities => "Qualities",
            Step::ActiveSkills => "Active Skills",
            Step::KnowledgeSkills => "Knowledge Skills",
            Step::Spells => "Spells & Spirits",
            Step::AdeptPowers => "Adept Powers",
            Step::ComplexForms => "Complex Forms",
            Step::Cyberware => "Cyberware & Bioware",
            Step::Gear => "Street Gear",
            Step::Vehicles => "Vehicles & Drones",
            Step::Contacts => "Contacts",
            Step::CharacterInfo => "Character Info",
            Step::Review => "Review & Finish",
        }
    }

    /// The tab where the step happens.
    pub fn tab(self) -> IssueTab {
        match self {
            Step::Concept | Step::LifeModules | Step::Attributes | Step::SpecialAttributes | Step::Qualities | Step::Review => IssueTab::Common,
            Step::ActiveSkills | Step::KnowledgeSkills => IssueTab::Skills,
            Step::Spells => IssueTab::Magician,
            Step::AdeptPowers => IssueTab::Adept,
            Step::ComplexForms => IssueTab::Technomancer,
            Step::Cyberware => IssueTab::Cyberware,
            Step::Gear => IssueTab::StreetGear,
            Step::Vehicles => IssueTab::Vehicles,
            Step::Contacts => IssueTab::Relationships,
            Step::CharacterInfo => IssueTab::CharacterInfo,
        }
    }

    /// The parts of the character whose issues belong to this step.
    pub fn areas(self) -> &'static [Area] {
        match self {
            Step::Concept => &[Area::Metatype],
            Step::LifeModules => &[Area::LifeModules],
            Step::Attributes => &[Area::Attributes],
            Step::SpecialAttributes => &[Area::SpecialAttributes],
            Step::Qualities => &[Area::Qualities],
            Step::ActiveSkills => &[Area::ActiveSkills, Area::SkillGroups],
            Step::KnowledgeSkills => &[Area::KnowledgeSkills],
            Step::Spells => &[Area::Spells],
            Step::AdeptPowers => &[Area::AdeptPowers, Area::MartialArts],
            Step::ComplexForms => &[Area::ComplexForms],
            Step::Cyberware => &[Area::Cyberware],
            Step::Gear => &[Area::Gear, Area::Armor, Area::Weapons, Area::Lifestyles, Area::Nuyen],
            Step::Vehicles => &[Area::Vehicles],
            Step::Contacts => &[Area::Contacts],
            Step::CharacterInfo => &[Area::CharacterInfo],
            Step::Review => &[Area::Karma],
        }
    }

    /// Whether an issue is this step's. The review step collects every
    /// error and the karma total.
    pub fn owns(self, issue: &Issue) -> bool {
        self.areas().contains(&issue.area) || (self == Step::Review && issue.severity == Severity::Error)
    }

    /// The rulebook page explaining the step, as (book code, page), for
    /// a build method.
    pub fn source(self, build_method: &str) -> (&'static str, &'static str) {
        match (self, build_method) {
            (Step::Concept, "SumtoTen") => ("RF", "62"),
            (Step::Concept | Step::Attributes, "Karma") => ("RF", "64"),
            (Step::Concept | Step::LifeModules, "LifeModule") => ("RF", "65"),
            (Step::Qualities, "Karma" | "LifeModule") => ("RF", "64"),
            (Step::Concept, _) => ("SR5", "62"),
            (Step::Attributes | Step::SpecialAttributes, _) => ("SR5", "66"),
            (Step::LifeModules, _) => ("RF", "65"),
            (Step::Qualities, _) => ("SR5", "71"),
            (Step::ActiveSkills, _) => ("SR5", "88"),
            (Step::KnowledgeSkills, _) => ("SR5", "89"),
            (Step::Spells, _) => ("SR5", "68"),
            (Step::AdeptPowers, _) => ("SR5", "308"),
            (Step::ComplexForms, _) => ("SR5", "252"),
            (Step::Cyberware | Step::Gear | Step::Vehicles, _) => ("SR5", "94"),
            (Step::Contacts, _) => ("SR5", "98"),
            (Step::CharacterInfo, _) => ("SR5", "103"),
            (Step::Review, _) => ("SR5", "100"),
        }
    }

    /// What to do in this step, in plain words (English; a paraphrase of
    /// the rules, not book text).
    pub fn explanation(self, build_method: &str) -> &'static str {
        let karma = matches!(build_method, "Karma" | "LifeModule");
        match self {
            Step::Concept if build_method == "LifeModule" => {
                "Life Modules start with 750 Karma. Your metatype was bought in the New Character wizard; its cost is already taken from your Karma. Think about who your runner is and where they come from before choosing modules."
            }
            Step::Concept if karma => {
                "Point Buy starts with 800 Karma and buys everything with it. Your metatype was bought in the New Character wizard; its cost is already taken from your Karma. Decide what your runner should be good at before spending the rest."
            }
            Step::Concept if build_method == "SumtoTen" => {
                "Sum-to-Ten works like priorities, but the five priority letters only have to add up to ten points, so a letter may repeat. Your priorities and metatype were set in the New Character wizard. Check that the metatype and the special attribute points fit your concept."
            }
            Step::Concept => {
                "Priorities rank the five parts of your character (metatype, attributes, magic or resonance, skills, resources) from A to E, each letter used once. They and your metatype were set in the New Character wizard. Check that the metatype and the special attribute points fit your concept."
            }
            Step::LifeModules => {
                "Pick one module for each stage of your runner's life: nationality, formative years, teen years, further education and real life. Each module costs Karma and gives the attributes, skills and qualities that this past would teach."
            }
            Step::Attributes if karma => {
                "Attributes start at your metatype's minimum. Raise them with Karma in the Karma column; each level costs five times the new rating. Only one physical or mental attribute may be at its natural maximum."
            }
            Step::Attributes => {
                "Spend all your attribute points on the eight physical and mental attributes, in the Points column. You cannot keep them for later. Only one attribute may reach its natural maximum. Karma can raise attributes further."
            }
            Step::SpecialAttributes if karma => {
                "Edge, Magic and Resonance are raised with Karma too. Magic and Resonance only appear once you have the matching quality (Magician, Adept, Technomancer...)."
            }
            Step::SpecialAttributes => {
                "Special attribute points from your metatype priority go to Edge, and to Magic or Resonance if you have them. Points left here are lost when you finish."
            }
            Step::Qualities if karma => {
                "Qualities are talents and flaws. To use magic or resonance, buy the matching quality (Adept, Magician, Mystic Adept, Aspected Magician or Technomancer) first. Positive qualities cost Karma, negative ones give Karma; each side is limited."
            }
            Step::Qualities => {
                "Qualities are talents (positive, they cost Karma) and flaws (negative, they give Karma). Each side is capped, normally at 25 Karma. Some qualities ask for a choice, such as a mentor spirit."
            }
            Step::ActiveSkills if karma => {
                "Buy active skills and skill groups with Karma. A group raises all its skills together and is cheaper than buying them one by one. You may add one specialization per skill."
            }
            Step::ActiveSkills => {
                "Spend your skill points on active skills and your skill group points on skill groups. Skills cap at 6 during creation (7 with Aptitude). One specialization per skill costs one skill point."
            }
            Step::KnowledgeSkills => {
                "You get free knowledge points equal to (Intuition + Logic) × 2 for knowledge and language skills. Your native language is free. Points beyond the free ones come out of your skill points or Karma."
            }
            Step::Spells => {
                "Magicians choose a tradition, then their spells, rituals and preparations. The priority gives some for free; more cost 5 Karma each. Bound spirits and foci can be added here too."
            }
            Step::AdeptPowers => {
                "Adepts get power points equal to Magic and spend them on adept powers. Mystic adepts buy their power points with Karma. Unused power points are lost."
            }
            Step::ComplexForms => {
                "Technomancers choose complex forms; the priority gives some for free and more cost Karma. Registered sprites can be added here too."
            }
            Step::Cyberware => {
                "Augmentations cost nuyen and Essence. Essence must stay above zero, and lowering it also lowers Magic and Resonance. Availability is limited to 12 at creation (the Restricted Gear quality allows more)."
            }
            Step::Gear => {
                "Spend your starting nuyen on gear, armor, weapons and a lifestyle. Up to 10 Karma can be traded for 2,000¥ each in the Common tab. At most 5,000¥ carries over when you finish."
            }
            Step::Vehicles => {
                "Vehicles, drones and their mods come out of the same nuyen. Riggers will want a control rig from the Cyberware step."
            }
            Step::Contacts => {
                "Contacts have Connection (how much they can do) and Loyalty (how much they care). You get Charisma × 3 free points; anything more costs Karma. No contact may be worth more than 7 points at creation."
            }
            Step::CharacterInfo => {
                "Give your runner a street name, a look and a background. None of this costs anything, but it makes the character playable."
            }
            Step::Review => {
                "Check the remaining issues. Errors must be fixed before Finish creation; warnings are only reminders. Leftover Karma above the carry-over limit (normally 7) is lost."
            }
        }
    }
}

pub const ALL: [Step; 16] = [
    Step::Concept,
    Step::LifeModules,
    Step::Attributes,
    Step::SpecialAttributes,
    Step::Qualities,
    Step::ActiveSkills,
    Step::KnowledgeSkills,
    Step::Spells,
    Step::AdeptPowers,
    Step::ComplexForms,
    Step::Cyberware,
    Step::Gear,
    Step::Vehicles,
    Step::Contacts,
    Step::CharacterInfo,
    Step::Review,
];

/// The steps for a build method, keeping only the magic and resonance
/// steps the character can use.
pub fn steps_for(build_method: &str, ch: &Character) -> Vec<Step> {
    let karma = matches!(build_method, "Karma" | "LifeModule");
    let mut out = vec![Step::Concept];
    if build_method == "LifeModule" {
        out.push(Step::LifeModules);
    }
    if karma {
        // Magic and resonance come from qualities in karma builds.
        out.push(Step::Qualities);
    }
    out.extend([Step::Attributes, Step::SpecialAttributes]);
    if !karma {
        out.push(Step::Qualities);
    }
    out.extend([Step::ActiveSkills, Step::KnowledgeSkills]);
    if ch.is_magician() && ch.mag_enabled() {
        out.push(Step::Spells);
    }
    if ch.is_adept() && ch.mag_enabled() {
        out.push(Step::AdeptPowers);
    }
    if ch.res_enabled() {
        out.push(Step::ComplexForms);
    }
    out.extend([Step::Cyberware, Step::Gear, Step::Vehicles, Step::Contacts, Step::CharacterInfo, Step::Review]);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Done,
    Current,
    Upcoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepStatus {
    pub step: Step,
    pub state: State,
    pub errors: usize,
    pub warnings: usize,
}

impl StepStatus {
    /// Whether Next may leave the step.
    pub fn can_advance(&self) -> bool {
        self.errors == 0
    }
}

/// Status of each step: steps before the furthest one `reached` (other
/// than the current one) are done, and each counts its errors and
/// warnings.
pub fn status(steps: &[Step], current: usize, reached: usize, issues: &[Issue]) -> Vec<StepStatus> {
    let reached = reached.max(current);
    steps
        .iter()
        .enumerate()
        .map(|(i, &step)| {
            let mine = issues.iter().filter(|x| step.owns(x));
            let (mut errors, mut warnings) = (0, 0);
            for x in mine {
                match x.severity {
                    Severity::Error => errors += 1,
                    Severity::Warning => warnings += 1,
                    Severity::Info => {}
                }
            }
            let state = if i == current {
                State::Current
            } else if i < reached {
                State::Done
            } else {
                State::Upcoming
            };
            StepStatus { step, state, errors, warnings }
        })
        .collect()
}

/// Where to start (or resume) the guide: the first step with an error,
/// else the first with a warning, else the review. The concept step,
/// already done in the wizard, is skipped unless it has an error.
pub fn suggested(steps: &[Step], issues: &[Issue]) -> usize {
    let has = |sev: Severity| steps.iter().position(|s| issues.iter().any(|i| i.severity == sev && s.owns(i) && (*s != Step::Review)));
    has(Severity::Error).or_else(|| has(Severity::Warning)).unwrap_or(steps.len().saturating_sub(1))
}
