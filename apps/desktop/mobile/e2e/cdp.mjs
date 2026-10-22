const CALL_TIMEOUT_MS = 30_000;
const APP_PAGE = /tauri\.localhost/;

export async function findAppPage(port) {
  const res = await fetch(`http://127.0.0.1:${port}/json`);
  const pages = await res.json();
  const page = pages.find(p => p.type === "page" && APP_PAGE.test(p.url)) ?? pages.find(p => p.type === "page");
  if (!page) throw new Error(`no app page behind debug port ${port}`);
  return page.webSocketDebuggerUrl;
}

export async function connect(port) {
  const socket = new WebSocket(await findAppPage(port));
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error(`could not open the app page on port ${port}`)), { once: true });
  });
  let next = 1;
  const waiting = new Map();
  socket.addEventListener("message", event => {
    const reply = JSON.parse(event.data);
    const settle = waiting.get(reply.id);
    if (!settle) return;
    waiting.delete(reply.id);
    settle(reply);
  });

  const evaluate = expression =>
    new Promise((resolve, reject) => {
      const id = next++;
      const timer = setTimeout(() => {
        waiting.delete(id);
        reject(new Error("the app did not answer in time"));
      }, CALL_TIMEOUT_MS);
      waiting.set(id, reply => {
        clearTimeout(timer);
        const result = reply.result?.result;
        const thrown = reply.result?.exceptionDetails;
        if (reply.error || thrown) reject(new Error(reply.error?.message ?? thrown.exception?.description ?? thrown.text));
        else resolve(result?.value);
      });
      socket.send(JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }));
    });

  return {
    evaluate,
    async invoke(command, args = {}) {
      const call = `window.__TAURI__.core.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})`;
      const json = await evaluate(`(async () => JSON.stringify(await ${call}))()`);
      return json === undefined ? undefined : JSON.parse(json);
    },
    close() {
      socket.close();
    },
  };
}
