# Commy Web

The commy.chat website and docs: Next.js (App Router, TypeScript), no
database of its own. Accounts live in the Better Auth server in `../auth`;
this site renders the account and device approval pages and calls that
service at `/api/auth` on the same origin.

Production runs through the stack in `../deploy` (see `../deploy/HOSTING.md`),
where Caddy routes `/api/auth/*` to the auth service and everything else here.

## Pages

- `/` home, `/download`, `/docs/*`
- `/account` sign in (Google, email and password, magic link) and linked devices
- `/device` approves a desktop app sign-in code (OAuth device flow)

## Local development

Run the full stack from `../deploy` for anything touching sign-in, since the
browser must reach `/api/auth` on the same origin as the site. For pure
content work, `npm install && npm run dev` serves the site at
http://localhost:3000; account pages then show as signed out.

## Scripts

- `npm run dev` - development server
- `npm run build` - production build
- `npm run start` - serve the production build
- `npm run lint` - ESLint
