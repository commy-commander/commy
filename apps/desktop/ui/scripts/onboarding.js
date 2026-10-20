const ONBOARDING_LIMITS = {
  questions: 5,
  minAnswers: 2,
  answers: 10,
  rolesPerAnswer: 5,
  roleRefs: 40,
  titleChars: 80,
  descriptionChars: 160,
  answerChars: 60,
  textBytes: 2000,
};

const NEVER_HANDED_OUT = ["administrator", "manage_server", "manage_roles"];
const QUESTIONS_PER_SINGLE_CARD = 3;

function onboardingRoles(roles) {
  return (roles || []).filter(r => !(r.permissions || []).some(p => NEVER_HANDED_OUT.includes(p)));
}

function cleanOnboardingText(raw, maxChars) {
  const words = String(raw || "").split(/\s+/).filter(Boolean).join(" ");
  const kept = [...words].filter(c => !/\p{Cc}/u.test(c)).slice(0, maxChars).join("");
  return kept.trimEnd();
}

function tidyAnswer(answer, allowed) {
  const label = cleanOnboardingText(answer && answer.label, ONBOARDING_LIMITS.answerChars);
  if (!label) return null;
  const roles = [];
  for (const role of (answer && answer.roles) || []) {
    if (!allowed.has(role) || roles.includes(role) || roles.length >= ONBOARDING_LIMITS.rolesPerAnswer) continue;
    roles.push(role);
  }
  return { label, roles };
}

function tidyQuestion(question, allowed) {
  const title = cleanOnboardingText(question && question.title, ONBOARDING_LIMITS.titleChars);
  if (!title) return null;
  const answers = ((question && question.answers) || []).map(a => tidyAnswer(a, allowed)).filter(Boolean).slice(0, ONBOARDING_LIMITS.answers);
  if (answers.length < ONBOARDING_LIMITS.minAnswers) return null;
  const description = cleanOnboardingText(question.description, ONBOARDING_LIMITS.descriptionChars);
  return { title, description, multiple: Boolean(question.multiple), answers };
}

function tidyOnboarding(draft, roleIds) {
  const allowed = new Set(roleIds || []);
  const questions = ((draft && draft.questions) || []).map(q => tidyQuestion(q, allowed)).filter(Boolean).slice(0, ONBOARDING_LIMITS.questions);
  return questions.length ? { questions } : null;
}

function onboardingProblems(draft) {
  const problems = [];
  const questions = (draft && draft.questions) || [];
  if (!questions.length) problems.push("Add at least one question");
  if (questions.length > ONBOARDING_LIMITS.questions) problems.push(`Keep it to ${ONBOARDING_LIMITS.questions} questions`);
  questions.forEach((q, i) => {
    const n = i + 1;
    if (!cleanOnboardingText(q.title, ONBOARDING_LIMITS.titleChars)) problems.push(`Question ${n} needs a title`);
    const answers = (q.answers || []).filter(a => cleanOnboardingText(a.label, ONBOARDING_LIMITS.answerChars));
    if (answers.length < ONBOARDING_LIMITS.minAnswers) problems.push(`Question ${n} needs at least ${ONBOARDING_LIMITS.minAnswers} answers`);
    if (answers.length > ONBOARDING_LIMITS.answers) problems.push(`Question ${n} can have at most ${ONBOARDING_LIMITS.answers} answers`);
  });
  const usage = onboardingUsage(draft);
  if (usage.bytes > ONBOARDING_LIMITS.textBytes) problems.push(`Keep the text under ${ONBOARDING_LIMITS.textBytes} characters in all`);
  if (usage.roles > ONBOARDING_LIMITS.roleRefs) problems.push(`Keep it to ${ONBOARDING_LIMITS.roleRefs} roles across every answer`);
  return problems;
}

function utf8Bytes(text) {
  return new TextEncoder().encode(String(text || "")).length;
}

function onboardingUsage(draft) {
  let bytes = 0;
  let roles = 0;
  for (const q of (draft && draft.questions) || []) {
    bytes += utf8Bytes(cleanOnboardingText(q.title, ONBOARDING_LIMITS.titleChars));
    bytes += utf8Bytes(cleanOnboardingText(q.description, ONBOARDING_LIMITS.descriptionChars));
    for (const a of q.answers || []) {
      bytes += utf8Bytes(cleanOnboardingText(a.label, ONBOARDING_LIMITS.answerChars));
      roles += (a.roles || []).length;
    }
  }
  return { bytes, roles };
}

function onboardingPages(count) {
  const n = Math.max(0, Number(count) || 0);
  if (n <= QUESTIONS_PER_SINGLE_CARD) return n ? [Array.from({ length: n }, (_, i) => i)] : [];
  return Array.from({ length: n }, (_, i) => [i]);
}

function answerHeld(answer, held) {
  const roles = (answer && answer.roles) || [];
  return roles.length > 0 && roles.every(r => held.has(r));
}

function inferPicks(onboarding, heldRoles) {
  const held = new Set(heldRoles || []);
  return ((onboarding && onboarding.questions) || []).map(q => {
    const picked = (q.answers || []).map((a, i) => (answerHeld(a, held) ? i : -1)).filter(i => i >= 0);
    return q.multiple ? picked : picked.slice(0, 1);
  });
}

function togglePick(question, picked, index) {
  const current = picked || [];
  if (current.includes(index)) return current.filter(i => i !== index);
  return question && question.multiple ? [...current, index].sort((a, b) => a - b) : [index];
}

function pickedRoles(onboarding, picks) {
  const out = [];
  ((onboarding && onboarding.questions) || []).forEach((q, qi) => {
    for (const ai of (picks && picks[qi]) || []) {
      for (const role of ((q.answers || [])[ai] || {}).roles || []) {
        if (!out.includes(role)) out.push(role);
      }
    }
  });
  return out;
}

function onboardingRoleIds(onboarding) {
  const out = [];
  for (const q of (onboarding && onboarding.questions) || []) {
    for (const a of q.answers || []) {
      for (const role of a.roles || []) if (!out.includes(role)) out.push(role);
    }
  }
  return out;
}

function answerDiff(onboarding, picks, heldRoles) {
  const held = new Set(heldRoles || []);
  const wanted = new Set(pickedRoles(onboarding, picks));
  const grant = [...wanted].filter(r => !held.has(r));
  const revoke = onboardingRoleIds(onboarding).filter(r => held.has(r) && !wanted.has(r));
  return { grant, revoke };
}

function unansweredQuestions(onboarding, picks) {
  return ((onboarding && onboarding.questions) || []).map((_, i) => i).filter(i => !((picks && picks[i]) || []).length);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    ONBOARDING_LIMITS,
    NEVER_HANDED_OUT,
    QUESTIONS_PER_SINGLE_CARD,
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
  };
}
