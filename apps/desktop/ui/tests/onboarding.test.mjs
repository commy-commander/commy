import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  ONBOARDING_LIMITS,
  NEVER_HANDED_OUT,
  onboardingRoles,
  cleanOnboardingText,
  tidyOnboarding,
  onboardingProblems,
  onboardingUsage,
  onboardingPages,
  inferPicks,
  togglePick,
  pickedRoles,
  onboardingRoleIds,
  answerDiff,
  unansweredQuestions,
} = require("../scripts/onboarding.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const rustSource = () => readFileSync(join(repo, "crates", "commy-core", "src", "onboarding.rs"), "utf8");
const rustConst = name => Number(new RegExp(`pub const ${name}: usize = (\\d+);`).exec(rustSource())[1]);

const answer = (label, roles = []) => ({ label, roles });
const question = (title, answers, extra = {}) => ({ title, description: "", multiple: false, answers, ...extra });
const twoQuestions = {
  questions: [
    question("What do you play?", [answer("Shooters", ["r1"]), answer("Strategy", ["r2"]), answer("A bit of everything", ["r1", "r2"])], { multiple: true }),
    question("Pings?", [answer("Everything", ["r3"]), answer("Only events", [])]),
  ],
};

test("the limits match the core so the editor refuses what the store would", () => {
  assert.equal(ONBOARDING_LIMITS.questions, rustConst("MAX_QUESTIONS"));
  assert.equal(ONBOARDING_LIMITS.minAnswers, rustConst("MIN_ANSWERS"));
  assert.equal(ONBOARDING_LIMITS.answers, rustConst("MAX_ANSWERS"));
  assert.equal(ONBOARDING_LIMITS.rolesPerAnswer, rustConst("MAX_ROLES_PER_ANSWER"));
  assert.equal(ONBOARDING_LIMITS.roleRefs, rustConst("MAX_ROLE_REFS"));
  assert.equal(ONBOARDING_LIMITS.titleChars, rustConst("MAX_TITLE_CHARS"));
  assert.equal(ONBOARDING_LIMITS.descriptionChars, rustConst("MAX_DESCRIPTION_CHARS"));
  assert.equal(ONBOARDING_LIMITS.answerChars, rustConst("MAX_ANSWER_CHARS"));
  assert.equal(ONBOARDING_LIMITS.textBytes, rustConst("MAX_TEXT_BYTES"));
});

test("roles that run the server are never offered to an answer", () => {
  const roles = [
    { id: "a", name: "Gamer", permissions: [] },
    { id: "b", name: "Mod", permissions: ["manage_messages", "ban_members"] },
    { id: "c", name: "Admin", permissions: ["administrator"] },
    { id: "d", name: "Staff", permissions: ["manage_server"] },
    { id: "e", name: "Role boss", permissions: ["manage_roles"] },
  ];
  assert.deepEqual(onboardingRoles(roles).map(r => r.id), ["a", "b"]);
  assert.deepEqual(NEVER_HANDED_OUT, ["administrator", "manage_server", "manage_roles"]);
  assert.deepEqual(onboardingRoles(null), []);
});

test("text is trimmed, collapsed and clipped like the core does it", () => {
  assert.equal(cleanOnboardingText("  What   do\nyou\tplay? "), "What do you play?");
  assert.equal(cleanOnboardingText("Yes\u0007!", 10), "Yes!");
  assert.equal(cleanOnboardingText("x".repeat(90), ONBOARDING_LIMITS.titleChars).length, ONBOARDING_LIMITS.titleChars);
  assert.equal(cleanOnboardingText(null, 5), "");
});

test("tidying drops unknown roles, empty answers and questions that cannot stand", () => {
  const draft = {
    questions: [
      question("  Pick  ", [answer("A", ["r1", "gone", "r1"]), answer("  ", []), answer("B", [])]),
      question("", [answer("A"), answer("B")]),
      question("Lonely", [answer("A")]),
    ],
  };
  const tidy = tidyOnboarding(draft, ["r1"]);
  assert.deepEqual(tidy, { questions: [{ title: "Pick", description: "", multiple: false, answers: [answer("A", ["r1"]), answer("B")] }] });
  assert.equal(tidyOnboarding({ questions: [] }, []), null);
  assert.equal(tidyOnboarding(null, []), null);
  const many = { questions: Array.from({ length: 7 }, (_, i) => question(`q${i}`, [answer("A"), answer("B")])) };
  assert.equal(tidyOnboarding(many, []).questions.length, ONBOARDING_LIMITS.questions);
  const crowded = question("q", [answer("A", ["1", "2", "3", "4", "5", "6", "7"]), answer("B")]);
  assert.equal(tidyOnboarding({ questions: [crowded] }, ["1", "2", "3", "4", "5", "6", "7"]).questions[0].answers[0].roles.length, ONBOARDING_LIMITS.rolesPerAnswer);
});

test("problems name the question that needs work", () => {
  assert.deepEqual(onboardingProblems(twoQuestions), []);
  assert.deepEqual(onboardingProblems({ questions: [] }), ["Add at least one question"]);
  const bad = { questions: [question("", [answer("A")]), question("Fine", [answer("A"), answer("B")])] };
  assert.deepEqual(onboardingProblems(bad), ["Question 1 needs a title", "Question 1 needs at least 2 answers"]);
  const long = { questions: [question("t".repeat(80), Array.from({ length: 10 }, (_, i) => answer(`${i}${"a".repeat(59)}`)))] };
  long.questions.push(...Array.from({ length: 4 }, () => structuredClone(long.questions[0])));
  assert.ok(onboardingProblems(long).includes(`Keep the text under ${ONBOARDING_LIMITS.textBytes} characters in all`));
  const refs = { questions: Array.from({ length: 5 }, () => question("q", Array.from({ length: 10 }, (_, i) => answer(`a${i}`, ["1", "2", "3", "4", "5"])))) };
  assert.ok(onboardingProblems(refs).includes(`Keep it to ${ONBOARDING_LIMITS.roleRefs} roles across every answer`));
});

test("usage counts bytes of the tidy text and every role reference", () => {
  assert.deepEqual(onboardingUsage(twoQuestions), { bytes: "What do you play?".length + "Shooters".length + "Strategy".length + "A bit of everything".length + "Pings?".length + "Everything".length + "Only events".length, roles: 5 });
  assert.equal(onboardingUsage({ questions: [question("  héllo  ", [])] }).bytes, 6);
  assert.deepEqual(onboardingUsage(null), { bytes: 0, roles: 0 });
});

test("three questions or fewer share one card, more get a card each", () => {
  assert.deepEqual(onboardingPages(0), []);
  assert.deepEqual(onboardingPages(1), [[0]]);
  assert.deepEqual(onboardingPages(3), [[0, 1, 2]]);
  assert.deepEqual(onboardingPages(4), [[0], [1], [2], [3]]);
  assert.deepEqual(onboardingPages(5), [[0], [1], [2], [3], [4]]);
});

test("held roles tell which answers were picked before", () => {
  assert.deepEqual(inferPicks(twoQuestions, ["r1"]), [[0], []]);
  assert.deepEqual(inferPicks(twoQuestions, ["r1", "r2", "r3"]), [[0, 1, 2], [0]]);
  assert.deepEqual(inferPicks(twoQuestions, []), [[], []]);
  const single = { questions: [question("q", [answer("A", ["r1"]), answer("B", ["r1"])])] };
  assert.deepEqual(inferPicks(single, ["r1"]), [[0]], "a single choice keeps one answer");
  assert.deepEqual(inferPicks(null, ["r1"]), []);
});

test("picking toggles one answer for single choice and any number for multiple", () => {
  const single = question("q", [answer("A"), answer("B")]);
  const multi = question("q", [answer("A"), answer("B"), answer("C")], { multiple: true });
  assert.deepEqual(togglePick(single, [], 1), [1]);
  assert.deepEqual(togglePick(single, [1], 0), [0]);
  assert.deepEqual(togglePick(single, [0], 0), []);
  assert.deepEqual(togglePick(multi, [2], 0), [0, 2]);
  assert.deepEqual(togglePick(multi, [0, 2], 2), [0]);
  assert.deepEqual(togglePick(multi, null, 1), [1]);
});

test("the diff grants what the picks want and takes back what the questions gave before", () => {
  assert.deepEqual(pickedRoles(twoQuestions, [[2], [1]]), ["r1", "r2"]);
  assert.deepEqual(onboardingRoleIds(twoQuestions), ["r1", "r2", "r3"]);
  assert.deepEqual(answerDiff(twoQuestions, [[0], [0]], []), { grant: ["r1", "r3"], revoke: [] });
  assert.deepEqual(answerDiff(twoQuestions, [[1], [1]], ["r1", "r3", "mod"]), { grant: ["r2"], revoke: ["r1", "r3"] });
  assert.deepEqual(answerDiff(twoQuestions, [[0], []], ["r1"]), { grant: [], revoke: [] });
  assert.deepEqual(answerDiff(twoQuestions, [[], []], ["mod"]), { grant: [], revoke: [] }, "roles the questions never hand out are left alone");
  assert.deepEqual(unansweredQuestions(twoQuestions, [[0], []]), [1]);
  assert.deepEqual(unansweredQuestions(twoQuestions, [[0], [1]]), []);
});
