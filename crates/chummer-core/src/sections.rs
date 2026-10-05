//! Item lists in a `.chum5` that are shown as tables: gear, ware, weapons,
//! spells, contacts and so on. Each section names its XML container, item
//! element and the columns worth showing. Nested items (gear in gear, mods
//! in vehicles) live under `<children>` or similar and are walked by
//! [`Section::child_containers`].

#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub header: &'static str,
    pub field: &'static str,
}

const fn c(header: &'static str, field: &'static str) -> Column {
    Column { header, field }
}

#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub label: &'static str,
    pub container: &'static str,
    pub item: &'static str,
    pub columns: &'static [Column],
    /// Child containers that hold nested items of a renderable kind:
    /// (container, item).
    pub child_containers: &'static [(&'static str, &'static str)],
    /// Data file that defines these items, for translations and lookups.
    pub data_file: &'static str,
}

pub const QUALITIES: Section = Section {
    label: "Qualities",
    container: "qualities",
    item: "quality",
    columns: &[c("Name", "name"), c("Extra", "extra"), c("Type", "qualitytype"), c("Karma", "bp"), c("Source", "source"), c("Page", "page")],
    child_containers: &[],
    data_file: "qualities.xml",
};

pub const CONTACTS: Section = Section {
    label: "Contacts",
    container: "contacts",
    item: "contact",
    columns: &[c("Name", "name"), c("Role", "role"), c("Location", "location"), c("Connection", "connection"), c("Loyalty", "loyalty"), c("Type", "type")],
    child_containers: &[],
    data_file: "contacts.xml",
};

pub const SPELLS: Section = Section {
    label: "Spells",
    container: "spells",
    item: "spell",
    columns: &[c("Name", "name"), c("Category", "category"), c("Type", "type"), c("Range", "range"), c("Duration", "duration"), c("DV", "dv"), c("Source", "source")],
    child_containers: &[],
    data_file: "spells.xml",
};

pub const POWERS: Section = Section {
    label: "Adept Powers",
    container: "powers",
    item: "power",
    columns: &[c("Name", "name"), c("Extra", "extra"), c("Rating", "rating"), c("PP/Level", "pointsperlevel"), c("Action", "action"), c("Source", "source")],
    child_containers: &[],
    data_file: "powers.xml",
};

pub const COMPLEX_FORMS: Section = Section {
    label: "Complex Forms",
    container: "complexforms",
    item: "complexform",
    columns: &[c("Name", "name"), c("Target", "target"), c("Duration", "duration"), c("FV", "fv"), c("Source", "source")],
    child_containers: &[],
    data_file: "complexforms.xml",
};

pub const SPIRITS: Section = Section {
    label: "Spirits & Sprites",
    container: "spirits",
    item: "spirit",
    columns: &[c("Name", "name"), c("Given name", "crittername"), c("Force", "force"), c("Services", "services"), c("Bound", "bound"), c("Type", "type")],
    child_containers: &[],
    data_file: "critters.xml",
};

pub const CRITTER_POWERS: Section = Section {
    label: "Critter Powers",
    container: "critterpowers",
    item: "critterpower",
    columns: &[c("Name", "name"), c("Extra", "extra"), c("Rating", "rating"), c("Type", "type"), c("Action", "action"), c("Range", "range")],
    child_containers: &[],
    data_file: "critterpowers.xml",
};

pub const AI_PROGRAMS: Section = Section {
    label: "Advanced Programs",
    container: "aiprograms",
    item: "aiprogram",
    columns: &[c("Name", "name"), c("Extra", "extra"), c("Source", "source"), c("Page", "page")],
    child_containers: &[],
    data_file: "programs.xml",
};

pub const METAMAGICS: Section = Section {
    label: "Metamagic & Echoes",
    container: "metamagics",
    item: "metamagic",
    columns: &[c("Name", "name"), c("Grade", "grade"), c("Source", "source"), c("Page", "page")],
    child_containers: &[],
    data_file: "metamagic.xml",
};

pub const MARTIAL_ARTS: Section = Section {
    label: "Martial Arts",
    container: "martialarts",
    item: "martialart",
    columns: &[c("Name", "name"), c("Rating", "rating"), c("Source", "source"), c("Page", "page")],
    child_containers: &[("martialarttechniques", "martialarttechnique")],
    data_file: "martialarts.xml",
};

pub const CYBERWARE: Section = Section {
    label: "Cyberware & Bioware",
    container: "cyberwares",
    item: "cyberware",
    columns: &[c("Name", "name"), c("Rating", "rating"), c("Grade", "grade"), c("Essence", "ess"), c("Capacity", "capacity"), c("Avail", "avail"), c("Cost", "cost"), c("Location", "location")],
    child_containers: &[("children", "cyberware"), ("gears", "gear")],
    data_file: "cyberware.xml",
};

pub const ARMOR: Section = Section {
    label: "Armor",
    container: "armors",
    item: "armor",
    columns: &[c("Name", "name"), c("Armor", "armor"), c("Capacity", "armorcapacity"), c("Equipped", "equipped"), c("Avail", "avail"), c("Cost", "cost")],
    child_containers: &[("armormods", "armormod"), ("gears", "gear")],
    data_file: "armor.xml",
};

pub const WEAPONS: Section = Section {
    label: "Weapons",
    container: "weapons",
    item: "weapon",
    columns: &[
        c("Name", "name"),
        c("Category", "category"),
        c("Acc", "accuracy"),
        c("DV", "damage"),
        c("AP", "ap"),
        c("Mode", "mode"),
        c("RC", "rc"),
        c("Ammo", "ammo"),
        c("Avail", "avail"),
        c("Cost", "cost"),
    ],
    child_containers: &[("accessories", "accessory"), ("underbarrel", "weapon")],
    data_file: "weapons.xml",
};

pub const GEAR: Section = Section {
    label: "Gear",
    container: "gears",
    item: "gear",
    columns: &[c("Name", "name"), c("Category", "category"), c("Rating", "rating"), c("Qty", "qty"), c("Avail", "avail"), c("Cost", "cost"), c("Location", "location")],
    child_containers: &[("children", "gear")],
    data_file: "gear.xml",
};

pub const VEHICLES: Section = Section {
    label: "Vehicles & Drones",
    container: "vehicles",
    item: "vehicle",
    columns: &[
        c("Name", "name"),
        c("Category", "category"),
        c("Handling", "handling"),
        c("Speed", "speed"),
        c("Accel", "accel"),
        c("Body", "body"),
        c("Armor", "armor"),
        c("Pilot", "pilot"),
        c("Sensor", "sensor"),
        c("Cost", "cost"),
    ],
    child_containers: &[("mods", "mod"), ("weapons", "weapon"), ("gears", "gear")],
    data_file: "vehicles.xml",
};

pub const LIFESTYLES: Section = Section {
    label: "Lifestyles",
    container: "lifestyles",
    item: "lifestyle",
    columns: &[c("Name", "name"), c("Lifestyle", "baselifestyle"), c("Cost", "cost"), c("Months", "months"), c("Roommates", "roommates"), c("Increment", "increment")],
    child_containers: &[],
    data_file: "lifestyles.xml",
};

pub const EXPENSES: Section = Section {
    label: "Karma & Nuyen Log",
    container: "expenses",
    item: "expense",
    columns: &[c("Date", "date"), c("Type", "type"), c("Amount", "amount"), c("Reason", "reason")],
    child_containers: &[],
    data_file: "",
};

pub const MAGIC: &[Section] = &[SPELLS, POWERS, SPIRITS, METAMAGICS, CRITTER_POWERS];
pub const EQUIPMENT: &[Section] = &[GEAR, CYBERWARE, ARMOR, WEAPONS, VEHICLES, LIFESTYLES];
