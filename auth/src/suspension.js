import { APIError } from "better-auth/api";

export const SUSPENDED_CODE = "ACCOUNT_SUSPENDED";
export const REASON_MAX = 200;
const IDENTITY_MODEL = "commyIdentity";
const COMMY_USER = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export const suspensionFields = {
  suspendedAt: { type: "date", required: false, input: false, returned: false },
  suspendedReason: { type: "string", required: false, input: false, returned: false },
};

export const isSuspended = (user) => Boolean(user?.suspendedAt);

export function cleanCommyUser(value) {
  const text = String(value ?? "").trim().toLowerCase();
  return COMMY_USER.test(text) ? text : null;
}

export function cleanReason(value) {
  const reason = String(value ?? "").replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim();
  return reason.length >= 1 && reason.length <= REASON_MAX ? reason : null;
}

export function suspendedMessage(abuseContact) {
  const where = abuseContact ? `write to ${abuseContact}` : "write to this account server's contact";
  return `This account is suspended for breaking the Terms. If you think this is a mistake, ${where}.`;
}

export function suspendedRefusal(abuseContact) {
  return new APIError("FORBIDDEN", { code: SUSPENDED_CODE, message: suspendedMessage(abuseContact) });
}

export function refuseSuspendedSessions({ abuseContact }) {
  return async (session, ctx) => {
    const internalAdapter = ctx?.context?.internalAdapter;
    if (!internalAdapter) throw new APIError("INTERNAL_SERVER_ERROR", { message: "Sessions can only start through the account server" });
    if (isSuspended(await internalAdapter.findUserById(session.userId))) throw suspendedRefusal(abuseContact);
  };
}

export async function rememberCommyUser(adapter, userId, commyUser, now = new Date()) {
  const where = [
    { field: "commyUser", value: commyUser },
    { field: "userId", value: userId },
  ];
  const known = await adapter.findOne({ model: IDENTITY_MODEL, where });
  if (known) await adapter.update({ model: IDENTITY_MODEL, where: [{ field: "id", value: known.id }], update: { seenAt: now } });
  else await adapter.create({ model: IDENTITY_MODEL, data: { commyUser, userId, seenAt: now } });
}

export async function accountsUsing(adapter, commyUser) {
  const rows = await adapter.findMany({ model: IDENTITY_MODEL, where: [{ field: "commyUser", value: commyUser }], limit: 100 });
  return [...new Set(rows.map((row) => row.userId))];
}

export async function commyUsersOf(adapter, userId) {
  const rows = await adapter.findMany({ model: IDENTITY_MODEL, where: [{ field: "userId", value: userId }], limit: 100 });
  return [...new Set(rows.map((row) => row.commyUser))];
}

export async function suspendedAmong(adapter, userIds) {
  const ids = [...new Set(userIds.filter(Boolean))];
  if (!ids.length) return new Set();
  const users = await adapter.findMany({ model: "user", where: [{ field: "id", operator: "in", value: ids }], limit: ids.length });
  return new Set(users.filter(isSuspended).map((user) => user.id));
}

export async function suspendAccount(internalAdapter, userId, reason, now = new Date()) {
  const user = await internalAdapter.updateUser(userId, { suspendedAt: now, suspendedReason: reason });
  await internalAdapter.deleteUserSessions(userId);
  return user;
}

export async function liftSuspension(internalAdapter, userId) {
  return internalAdapter.updateUser(userId, { suspendedAt: null, suspendedReason: null });
}

export const commySuspension = () => ({
  id: "commy-suspension",
  schema: {
    [IDENTITY_MODEL]: {
      fields: {
        commyUser: { type: "string", required: true, index: true },
        userId: {
          type: "string",
          required: true,
          references: { model: "user", field: "id", onDelete: "cascade" },
          index: true,
        },
        seenAt: { type: "date", required: true },
      },
    },
  },
});

export const OPERATOR_USAGE = `Usage: node src/suspension-cli.js <command>
  find <commy id|email>               the accounts behind a Commy id or email
  suspend <commy id|email> <reason>   sign them out everywhere and refuse new sign-ins
  lift <commy id|email>               let them sign in again
  list                                every suspended account`;

const LIST_SCAN = 100000;
const day = (value) => (value ? new Date(value).toISOString().slice(0, 16).replace("T", " ") : "-");

function describeAccount(user, commyUsers) {
  const state = isSuspended(user) ? `SUSPENDED ${day(user.suspendedAt)} (${user.suspendedReason ?? "no reason"})` : "active";
  return `${user.id}  ${user.email}  ${state}\n  Commy ids: ${commyUsers.length ? commyUsers.join(", ") : "none seen yet"}`;
}

async function accountsFor({ adapter, internalAdapter }, who) {
  const commyUser = cleanCommyUser(who);
  if (commyUser) {
    const users = await Promise.all((await accountsUsing(adapter, commyUser)).map((id) => internalAdapter.findUserById(id)));
    return users.filter(Boolean);
  }
  const email = String(who ?? "").trim().toLowerCase();
  if (!email.includes("@")) throw new Error(OPERATOR_USAGE);
  const found = await internalAdapter.findUserByEmail(email);
  return found?.user ? [found.user] : [];
}

async function described(context, users) {
  const lines = await Promise.all(users.map(async (user) => describeAccount(user, await commyUsersOf(context.adapter, user.id))));
  return `${lines.join("\n")}\n`;
}

const nobody = (who) => `No account here has used ${who}. A Commy id is matched once its app has signed in on version 1.0 or later.\n`;

export async function runOperatorCommand(context, [command, who, ...rest], now = new Date()) {
  if (command === "list") {
    const users = (await context.adapter.findMany({ model: "user", limit: LIST_SCAN })).filter(isSuspended);
    return users.length ? described(context, users) : "No suspended accounts.\n";
  }
  if (!["find", "suspend", "lift"].includes(command) || !who) throw new Error(OPERATOR_USAGE);
  const users = await accountsFor(context, who);
  if (!users.length) return nobody(who);
  if (command === "find") return described(context, users);
  if (command === "suspend") {
    const reason = cleanReason(rest.join(" "));
    if (!reason) throw new Error(`say why in up to ${REASON_MAX} characters, for example: suspend ${who} "harassment, report 3f2a"`);
    const done = await Promise.all(users.map((user) => suspendAccount(context.internalAdapter, user.id, reason, now)));
    return `Suspended ${done.length} account${done.length === 1 ? "" : "s"}. They are signed out everywhere, their Discover listings and invite links stop showing, and new sign-ins are refused.\n${await described(context, done)}`;
  }
  const done = await Promise.all(users.map((user) => liftSuspension(context.internalAdapter, user.id)));
  return `Lifted the suspension on ${done.length} account${done.length === 1 ? "" : "s"}. They can sign in again.\n${await described(context, done)}`;
}
