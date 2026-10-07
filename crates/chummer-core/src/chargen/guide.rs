//! Guided creation: the order in which to fill in a new character, one
//! part at a time, for each build method.
//!
//! The order follows the core rulebook's "Creating a shadowrunner" steps
//! (SR5 p. 62ff) for the priority system and Run Faster's construction
//! kits (p. 62ff) for Sum-to-Ten, Point Buy and Life Modules. Priorities,
//! metatype and the magic or resonance priority are chosen in the New
//! Character wizard, so the first step only reviews them. Each step lists
//! the [`Area`]s whose [issues](super::issues) it shows. A step is done
//! once the player has visited it and it has no errors or warnings left
//! ([`StepStatus::done`]); the GUI ticks off its tabs with that and
//! offers the next unfinished step ([`next`]).

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
    /// error and warning (what Finish creation will list) and the karma
    /// total.
    pub fn owns(self, issue: &Issue) -> bool {
        self.areas().contains(&issue.area) || (self == Step::Review && issue.severity != Severity::Info)
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

/// How far a step is: its open problems, and whether the player has been
/// there. Infos never count: they are suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepStatus {
    pub step: Step,
    pub errors: usize,
    pub warnings: usize,
    /// The player has looked at the step (it was the guide's step).
    pub visited: bool,
}

impl StepStatus {
    /// Errors and warnings left.
    pub fn open(&self) -> usize {
        self.errors + self.warnings
    }

    /// Ticked off: looked at, and nothing left to fix. A step without
    /// checks (vehicles, cyberware) is only done once it was visited.
    pub fn done(&self) -> bool {
        self.visited && self.open() == 0
    }
}

/// Status of each step, given the steps the player has visited.
pub fn status(steps: &[Step], visited: &[Step], issues: &[Issue]) -> Vec<StepStatus> {
    steps
        .iter()
        .map(|&step| {
            let (mut errors, mut warnings) = (0, 0);
            for x in issues.iter().filter(|x| step.owns(x)) {
                match x.severity {
                    Severity::Error => errors += 1,
                    Severity::Warning => warnings += 1,
                    Severity::Info => {}
                }
            }
            StepStatus { step, errors, warnings, visited: visited.contains(&step) }
        })
        .collect()
}

/// Where to start without a remembered position: the first step with an
/// error, else the first with a warning, else the review. The concept
/// step, already done in the wizard, is skipped unless it has an error.
/// The steps before it count as visited (see [`visited_before`]).
pub fn suggested(steps: &[Step], issues: &[Issue]) -> usize {
    let has = |sev: Severity| steps.iter().position(|s| issues.iter().any(|i| i.severity == sev && s.owns(i) && (*s != Step::Review)));
    has(Severity::Error).or_else(|| has(Severity::Warning)).unwrap_or(steps.len().saturating_sub(1))
}

/// The steps before `start`, which a new guide treats as already seen
/// (the wizard did the concept; an older file did the rest).
pub fn visited_before(steps: &[Step], start: usize) -> Vec<Step> {
    steps[..start.min(steps.len())].to_vec()
}

/// The step a tab's page is about: of the tab's steps (never the
/// review), the first with an error, else the first with a warning, else
/// the first not visited yet, else the first. `None` when the tab has no
/// step (Limits, Notes...).
pub fn focus(statuses: &[StepStatus], tab: IssueTab) -> Option<usize> {
    let mine: Vec<usize> = (0..statuses.len()).filter(|&i| statuses[i].step != Step::Review && statuses[i].step.tab() == tab).collect();
    let first = |f: &dyn Fn(&StepStatus) -> bool| mine.iter().copied().find(|&i| f(&statuses[i]));
    first(&|s| s.errors > 0).or_else(|| first(&|s| s.warnings > 0)).or_else(|| first(&|s| !s.visited)).or_else(|| mine.first().copied())
}

/// Where "Next" goes from step `from`: the next step that is not done,
/// else the review. `None` on the review itself.
pub fn next(statuses: &[StepStatus], from: usize) -> Option<usize> {
    let last = statuses.len().checked_sub(1)?;
    if from >= last {
        return None;
    }
    (from + 1..=last).find(|&i| !statuses[i].done()).or(Some(last))
}

/// Steps done and steps in all, without the review.
pub fn progress(statuses: &[StepStatus]) -> (usize, usize) {
    let steps = statuses.iter().filter(|s| s.step != Step::Review);
    let (done, total) = steps.fold((0, 0), |(d, t), s| (d + usize::from(s.done()), t + 1));
    (done, total)
}

/// A tab's checklist mark: `Some(true)` when every step on it is done,
/// `Some(false)` when some is not, `None` when the tab has no step.
pub fn tab_done(statuses: &[StepStatus], tab: IssueTab) -> Option<bool> {
    let mut mine = statuses.iter().filter(|s| s.step != Step::Review && s.step.tab() == tab).peekable();
    mine.peek()?;
    Some(mine.all(StepStatus::done))
}

impl Step {
    /// What is left in the step, errors first, then warnings, then
    /// suggestions (infos; not for the review, which lists only what
    /// Finish creation would).
    pub fn todo(self, issues: &[Issue]) -> Vec<&Issue> {
        let mut out: Vec<&Issue> = issues.iter().filter(|i| self.owns(i) && (self != Step::Review || i.severity != Severity::Info)).collect();
        out.sort_by_key(|i| i.severity);
        out
    }

    /// One line for the step when nothing is left to fix: what else can
    /// be done there (English; goes through `lang.tr`).
    pub fn prompt(self, build_method: &str) -> &'static str {
        let karma = matches!(build_method, "Karma" | "LifeModule");
        match self {
            Step::Concept => "Check that the metatype and priorities fit your concept.",
            Step::LifeModules => "Add a module for each stage of your runner's life.",
            Step::Attributes if karma => "Raise attributes with Karma.",
            Step::Attributes => "Karma can still raise attributes.",
            Step::SpecialAttributes => "Karma can still raise Edge, Magic or Resonance.",
            Step::Qualities => "Optional: add positive and negative qualities.",
            Step::ActiveSkills => "Karma can still raise skills or buy specializations.",
            Step::KnowledgeSkills => "Karma can still buy more knowledge skills.",
            Step::Spells => "Karma can buy more spells, at 5 Karma each.",
            Step::AdeptPowers => "Choose powers for your power points.",
            Step::ComplexForms => "Karma can buy more complex forms.",
            Step::Cyberware => "Optional: augmentations cost nuyen and Essence.",
            Step::Gear => "Buy weapons, armor, gear and a lifestyle with your nuyen.",
            Step::Vehicles => "Optional: vehicles and drones cost nuyen.",
            Step::Contacts => "Add the people your runner can call on.",
            Step::CharacterInfo => "Optional: a look and a background make the runner playable.",
            Step::Review => "Nothing blocks finishing. Finish creation switches to career mode.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chargen::issues::IssueKind;

    fn issue(severity: Severity, area: Area) -> Issue {
        Issue { severity, kind: IssueKind::AttributePointsLeft, area, item: None, args: vec!["1".into()] }
    }

    const STEPS: [Step; 6] = [Step::Concept, Step::Attributes, Step::Qualities, Step::ActiveSkills, Step::Vehicles, Step::Review];

    #[test]
    fn done_needs_a_visit_and_no_problems() {
        let issues = [issue(Severity::Warning, Area::Attributes), issue(Severity::Info, Area::Qualities)];
        let st = status(&STEPS, &[Step::Concept, Step::Attributes, Step::Qualities], &issues);
        assert!(st[0].done(), "visited and clean");
        assert!(!st[1].done(), "a warning is still open");
        assert!(st[2].done(), "infos do not count");
        assert!(!st[4].done(), "never visited: not ticked even without checks");
        assert_eq!((st[5].warnings, st[5].errors), (1, 0), "the review collects every warning");
        let st = status(&STEPS, &[Step::Attributes], &[issue(Severity::Error, Area::Attributes)]);
        assert!(!st[1].done());
        assert_eq!(st[1].open(), 1);
    }

    #[test]
    fn focus_picks_the_step_a_page_is_about() {
        let st = status(&STEPS, &[Step::Concept], &[]);
        // Common: concept is visited, attributes not yet.
        assert_eq!(focus(&st, IssueTab::Common), Some(1));
        let st = status(&STEPS, &[Step::Concept, Step::Attributes], &[issue(Severity::Warning, Area::Qualities)]);
        assert_eq!(focus(&st, IssueTab::Common), Some(2), "the step with something to do");
        let st = status(&STEPS, &[Step::Concept], &[issue(Severity::Warning, Area::Qualities), issue(Severity::Error, Area::Metatype)]);
        assert_eq!(focus(&st, IssueTab::Common), Some(0), "errors first");
        let st = status(&STEPS, &STEPS, &[]);
        assert_eq!(focus(&st, IssueTab::Common), Some(0), "all done: the first");
        assert_eq!(focus(&st, IssueTab::Skills), Some(3));
        assert_eq!(focus(&st, IssueTab::Relationships), None);
    }

    #[test]
    fn next_skips_done_steps_and_ends_at_the_review() {
        let st = status(&STEPS, &[Step::Concept, Step::Qualities], &[]);
        assert_eq!(next(&st, 0), Some(1));
        assert_eq!(next(&st, 1), Some(3), "qualities are done");
        let all = status(&STEPS, &STEPS, &[]);
        assert_eq!(next(&all, 1), Some(5), "nothing left: the review");
        assert_eq!(next(&all, 5), None);
        assert_eq!(progress(&st), (2, 5));
        assert_eq!(progress(&all), (5, 5));
    }

    #[test]
    fn tab_marks_and_todo_order() {
        let issues = [issue(Severity::Info, Area::Attributes), issue(Severity::Warning, Area::Attributes), issue(Severity::Error, Area::Attributes)];
        let st = status(&STEPS, &[Step::Concept, Step::Attributes, Step::Qualities, Step::ActiveSkills], &issues);
        assert_eq!(tab_done(&st, IssueTab::Common), Some(false));
        assert_eq!(tab_done(&st, IssueTab::Skills), Some(true));
        assert_eq!(tab_done(&st, IssueTab::Vehicles), Some(false));
        assert_eq!(tab_done(&st, IssueTab::Relationships), None);
        let todo = Step::Attributes.todo(&issues);
        assert_eq!(todo.iter().map(|i| i.severity).collect::<Vec<_>>(), [Severity::Error, Severity::Warning, Severity::Info]);
        assert_eq!(Step::Review.todo(&issues).len(), 2, "no infos in the review");
        assert_eq!(visited_before(&STEPS, 1), [Step::Concept]);
        for s in ALL {
            assert!(!s.prompt("Priority").is_empty() && !s.prompt("Karma").is_empty());
        }
    }
}
