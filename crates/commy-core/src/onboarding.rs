use crate::ids::{RoleId, UserId};
use crate::permissions::Permissions;
use crate::server::{Role, Server};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_QUESTIONS: usize = 5;
pub const MIN_ANSWERS: usize = 2;
pub const MAX_ANSWERS: usize = 10;
pub const MAX_ROLES_PER_ANSWER: usize = 5;
pub const MAX_ROLE_REFS: usize = 40;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_DESCRIPTION_CHARS: usize = 160;
pub const MAX_ANSWER_CHARS: usize = 60;
pub const MAX_TEXT_BYTES: usize = 2000;

const NEVER_HANDED_OUT: Permissions =
    Permissions(Permissions::ADMINISTRATOR.0 | Permissions::MANAGE_SERVER.0 | Permissions::MANAGE_ROLES.0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub label: String,
    #[serde(default)]
    pub roles: Vec<RoleId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub multiple: bool,
    pub answers: Vec<Answer>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Onboarding {
    pub questions: Vec<Question>,
}

pub fn hands_out_safely(role: &Role) -> bool {
    !role.is_everyone() && role.permissions.0 & NEVER_HANDED_OUT.0 == 0
}

pub fn grantable(roles: &[Role], role: RoleId) -> bool {
    roles.iter().any(|r| r.id == role && hands_out_safely(r))
}

pub fn may_self_assign(server: &Server, user: UserId, role: RoleId) -> bool {
    server.is_member(user)
        && grantable(&server.roles, role)
        && server.onboarding.as_ref().is_some_and(|o| o.grants(role))
}

fn clean_text(raw: &str, max_chars: usize) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn tidy_answer(answer: Answer, keep_role: &impl Fn(RoleId) -> bool) -> Option<Answer> {
    let label = clean_text(&answer.label, MAX_ANSWER_CHARS);
    if label.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let roles = answer
        .roles
        .into_iter()
        .filter(|r| keep_role(*r))
        .filter(|r| seen.insert(*r))
        .take(MAX_ROLES_PER_ANSWER)
        .collect();
    Some(Answer { label, roles })
}

fn tidy_question(question: Question, keep_role: &impl Fn(RoleId) -> bool) -> Option<Question> {
    let title = clean_text(&question.title, MAX_TITLE_CHARS);
    if title.is_empty() {
        return None;
    }
    let answers: Vec<Answer> = question
        .answers
        .into_iter()
        .filter_map(|a| tidy_answer(a, keep_role))
        .take(MAX_ANSWERS)
        .collect();
    if answers.len() < MIN_ANSWERS {
        return None;
    }
    let description = clean_text(&question.description, MAX_DESCRIPTION_CHARS);
    Some(Question { title, description, multiple: question.multiple, answers })
}

impl Onboarding {
    pub fn tidied(self, keep_role: impl Fn(RoleId) -> bool) -> Self {
        let questions = self
            .questions
            .into_iter()
            .filter_map(|q| tidy_question(q, &keep_role))
            .take(MAX_QUESTIONS)
            .collect();
        Self { questions }
    }

    pub fn text_bytes(&self) -> usize {
        self.questions
            .iter()
            .map(|q| q.title.len() + q.description.len() + q.answers.iter().map(|a| a.label.len()).sum::<usize>())
            .sum()
    }

    pub fn role_refs(&self) -> usize {
        self.questions.iter().flat_map(|q| &q.answers).map(|a| a.roles.len()).sum()
    }

    pub fn fits(&self) -> bool {
        self.text_bytes() <= MAX_TEXT_BYTES && self.role_refs() <= MAX_ROLE_REFS
    }

    pub fn valid(&self) -> bool {
        !self.questions.is_empty() && self.fits() && self.clone().tidied(|_| true) == *self
    }

    pub fn grants(&self, role: RoleId) -> bool {
        self.questions.iter().flat_map(|q| &q.answers).any(|a| a.roles.contains(&role))
    }

    pub fn roles(&self) -> Vec<RoleId> {
        let mut seen = HashSet::new();
        self.questions
            .iter()
            .flat_map(|q| &q.answers)
            .flat_map(|a| a.roles.iter().copied())
            .filter(|r| seen.insert(*r))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::test_support::{fixture, join};

    fn role(perms: Permissions) -> Role {
        Role {
            id: RoleId::new(),
            name: "r".into(),
            color: None,
            permissions: perms,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        }
    }

    fn answer(label: &str, roles: Vec<RoleId>) -> Answer {
        Answer { label: label.into(), roles }
    }

    fn question(title: &str, answers: Vec<Answer>) -> Question {
        Question { title: title.into(), description: String::new(), multiple: false, answers }
    }

    #[test]
    fn tidy_trims_text_collapses_whitespace_and_clips_lengths() {
        let long = "t".repeat(MAX_TITLE_CHARS + 20);
        let raw = Onboarding {
            questions: vec![Question {
                title: format!("  {long}  "),
                description: "one\n\ttwo   three".into(),
                multiple: true,
                answers: vec![answer("  Yes\u{7}  ", vec![]), answer(&"a".repeat(MAX_ANSWER_CHARS * 2), vec![])],
            }],
        };
        let tidy = raw.tidied(|_| true);
        let q = &tidy.questions[0];
        assert_eq!(q.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(q.description, "one two three");
        assert!(q.multiple);
        assert_eq!(q.answers[0].label, "Yes");
        assert_eq!(q.answers[1].label.chars().count(), MAX_ANSWER_CHARS);
        assert!(tidy.valid());
    }

    #[test]
    fn tidy_drops_empty_answers_short_questions_and_extra_ones() {
        let two = || vec![answer("a", vec![]), answer("b", vec![])];
        let questions = vec![
            question("", two()),
            question("one answer", vec![answer("a", vec![]), answer("  ", vec![])]),
            question("keep", two()),
            question("many", (0..MAX_ANSWERS + 3).map(|i| answer(&i.to_string(), vec![])).collect()),
        ];
        let mut more: Vec<Question> = (0..MAX_QUESTIONS).map(|i| question(&format!("q{i}"), two())).collect();
        let mut all = questions;
        all.append(&mut more);
        let tidy = Onboarding { questions: all }.tidied(|_| true);
        assert_eq!(tidy.questions.len(), MAX_QUESTIONS);
        assert_eq!(tidy.questions[0].title, "keep");
        assert_eq!(tidy.questions[1].answers.len(), MAX_ANSWERS);
        assert!(Onboarding::default().tidied(|_| true).questions.is_empty());
        assert!(!Onboarding::default().valid());
    }

    #[test]
    fn tidy_keeps_only_the_roles_it_is_told_to_and_dedupes_them() {
        let keep = RoleId::new();
        let drop = RoleId::new();
        let raw = Onboarding { questions: vec![question("q", vec![answer("a", vec![keep, drop, keep]), answer("b", vec![drop])])] };
        let tidy = raw.tidied(|r| r == keep);
        assert_eq!(tidy.questions[0].answers[0].roles, vec![keep]);
        assert!(tidy.questions[0].answers[1].roles.is_empty());
        assert_eq!(tidy.roles(), vec![keep]);
        assert!(tidy.grants(keep) && !tidy.grants(drop));
        let crowded = Onboarding {
            questions: vec![question("q", vec![answer("a", (0..MAX_ROLES_PER_ANSWER + 2).map(|_| RoleId::new()).collect()), answer("b", vec![])])],
        };
        assert_eq!(crowded.clone().tidied(|_| true).questions[0].answers[0].roles.len(), MAX_ROLES_PER_ANSWER);
        assert!(!crowded.valid(), "untidy input is not valid as is");
    }

    #[test]
    fn roles_that_run_the_server_are_never_handed_out() {
        let (mut state, _, _) = fixture();
        let plain = role(Permissions::NONE);
        let admin = role(Permissions::ADMINISTRATOR);
        let manager = role(Permissions::MANAGE_SERVER | Permissions::SEND_MESSAGES);
        let roles_admin = role(Permissions::MANAGE_ROLES);
        let moderator = role(Permissions::MANAGE_MESSAGES | Permissions::BAN_MEMBERS);
        assert!(hands_out_safely(&plain));
        assert!(hands_out_safely(&moderator));
        assert!(!hands_out_safely(&admin));
        assert!(!hands_out_safely(&manager));
        assert!(!hands_out_safely(&roles_admin));
        assert!(!hands_out_safely(&state.server.roles[0]), "not the everyone role");
        let sid = state.server.id;
        let unknown = RoleId::new();
        for r in [plain.clone(), admin.clone()] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: r }).unwrap();
        }
        assert!(grantable(&state.server.roles, plain.id));
        assert!(!grantable(&state.server.roles, admin.id));
        assert!(!grantable(&state.server.roles, unknown));
    }

    #[test]
    fn self_assignment_takes_a_member_a_safe_role_and_a_question_that_grants_it() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let plain = role(Permissions::NONE);
        let other = role(Permissions::NONE);
        let mut admin = role(Permissions::NONE);
        for r in [plain.clone(), other.clone(), admin.clone()] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: r }).unwrap();
        }
        assert!(!may_self_assign(&state.server, member, plain.id), "no questions yet");
        state.server.onboarding = Some(Onboarding {
            questions: vec![question("q", vec![answer("a", vec![plain.id]), answer("b", vec![admin.id])])],
        });
        assert!(may_self_assign(&state.server, member, plain.id));
        assert!(!may_self_assign(&state.server, member, other.id), "not in any answer");
        assert!(!may_self_assign(&state.server, UserId::new(), plain.id), "strangers");
        admin.permissions = Permissions::ADMINISTRATOR;
        state.apply(ServerEvent::RoleUpdate { server: sid, role: admin.clone() }).unwrap();
        assert!(!may_self_assign(&state.server, member, admin.id), "a role promoted later stops being handed out");
    }

    #[test]
    fn the_budget_caps_text_and_role_references() {
        let big = Onboarding {
            questions: (0..MAX_QUESTIONS)
                .map(|i| Question {
                    title: format!("{i}{}", "t".repeat(MAX_TITLE_CHARS - 1)),
                    description: "d".repeat(MAX_DESCRIPTION_CHARS),
                    multiple: false,
                    answers: (0..MAX_ANSWERS).map(|j| answer(&format!("{j}{}", "a".repeat(MAX_ANSWER_CHARS - 1)), vec![])).collect(),
                })
                .collect(),
        };
        assert!(big.text_bytes() > MAX_TEXT_BYTES);
        assert!(!big.valid());
        let refs = Onboarding {
            questions: (0..MAX_QUESTIONS)
                .map(|_| question("q", (0..MAX_ANSWERS).map(|_| answer("a", (0..MAX_ROLES_PER_ANSWER).map(|_| RoleId::new()).collect())).collect()))
                .collect(),
        };
        assert_eq!(refs.role_refs(), MAX_QUESTIONS * MAX_ANSWERS * MAX_ROLES_PER_ANSWER);
        assert!(!refs.valid());
        let small = Onboarding { questions: vec![question("q", vec![answer("a", vec![RoleId::new()]), answer("b", vec![])])] };
        assert!(small.valid());
        let json = serde_json::to_string(&small).unwrap();
        assert_eq!(serde_json::from_str::<Onboarding>(&json).unwrap(), small);
        assert_eq!(postcard::from_bytes::<Onboarding>(&postcard::to_stdvec(&small).unwrap()).unwrap(), small);
    }
}
