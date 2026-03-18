/// A runnable skill available from the skill picker.
#[derive(Clone)]
pub struct Skill {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub const SKILLS: &[Skill] = &[
    Skill {
        id: "daily_report",
        name: "Daily Activity Report",
        description: "Aggregate SOC data for the selected time window and generate a markdown report",
    },
];
