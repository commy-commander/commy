# Commy file storage

Commy shares files peer to peer: a picture or video someone posts loads from
the devices of members who have it, so it can be missing when none of them is
online. Big servers rarely notice, because someone who has the file is almost
always around; small servers, where everyone may be offline at once, notice
most. A server owner who wants every shared file to load at any time can
connect storage of their own. This folder is a small Cloudflare Worker that
keeps copies in an R2 bucket on the owner's Cloudflare account. The owner pays
Cloudflare for what their server stores; nobody else does.

The apps seal every file before it leaves the device, with a key that comes
from the file's content and only appears inside the server's encrypted
messages. The worker and the bucket only ever hold sealed copies with random
looking names, rounded up to a few fixed sizes, so neither the owner's
Cloudflare account nor Cloudflare can open them or tell what they are.

## What it costs

R2 charges US$0.015 per GB stored each month after the first 10 GB, and
nothing for downloads leaving it. Uploads and reads stay inside the free
monthly allowance (1 million writes, 10 million reads) for almost every server.
The Workers free plan covers 100,000 requests a day. See Cloudflare's
[R2 pricing](https://developers.cloudflare.com/r2/pricing/) and
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/).

## Set it up

1. Sign in to the Cloudflare dashboard and open **R2 Object Storage**. Turn R2
   on if it is not on yet.
2. Deploy the worker with the button below. It creates a bucket named
   `commy-files` and a worker named `commy-files` on your account.

   [![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/commy-commander/commy/tree/main/apps/file-store)

   Or from a copy of this folder: `npx wrangler r2 bucket create commy-files`
   then `npx wrangler deploy`.
3. Open the bucket's **Settings > Object lifecycle rules** and add a rule
   that deletes objects 90 days after upload, so storage stays small. Older
   files keep loading from members' devices.
4. Copy the worker's address, like `https://commy-files.yourname.workers.dev`.
   Opening it in a browser shows `{"commy_files":1,...}`.
5. In Commy, open **Server Settings > Integrations > Cloudflare Storage**, paste the
   address and choose **Save**. Commy checks that the address answers first.

From then on, files up to 30 MB posted in that server's channels are also kept
in the bucket, and members load them from there first. Direct messages and
bigger files stay peer to peer. Remove the address to go back to peer to peer
only; nothing already posted breaks.

The browser app at commy.chat reaches storage on a `workers.dev` address.
A worker on a custom domain works in the desktop and phone apps; in the
browser those files load peer to peer.

## Limits worth knowing

Anyone who can read the server's messages can learn the worker's address and
upload sealed files to it. Keep an eye on the bucket's size in the dashboard,
and set a [billing notification](https://developers.cloudflare.com/billing/manage/budget-alerts/)
if you want a warning. The worker never deletes or replaces a file it has
kept.

## Tests

`node --test test/*.test.mjs`
