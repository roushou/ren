import { test } from "node:test";
import assert from "node:assert/strict";
import { configure } from "./ren.mjs";

const session = "11111111-1111-4111-8111-111111111111";
const other = "22222222-2222-4222-8222-222222222222";

test("preserves native streaming/auth options and scopes attribution to a Pi run", () => {
  const handlers = new Map();
  const providers = new Map();
  const requests = [];
  const implementation = {
    streamSimple: (...args) => {
      requests.push(args);
      return "native-stream";
    },
  };
  const pi = {
    on: (name, handler) => handlers.set(name, handler),
    registerProvider: (name, value) => providers.set(name, value),
  };
  configure(pi, new URL("http://127.0.0.1:8787"), {
    anthropicMessagesApi: () => implementation,
    openAICodexResponsesApi: () => implementation,
    openAIResponsesApi: () => implementation,
  });
  const emit = (name, id = session) =>
    handlers.get(name)({}, { sessionManager: { getSessionId: () => id } });
  emit("session_start");
  emit("agent_start");
  const options = {
    apiKey: "fixture-token",
    headers: { authorization: "Bearer fixture-token" },
    sessionId: session,
    transport: "websocket",
    onPayload() {},
  };
  const context = { messages: [] };
  const model = {
    provider: "openai-codex",
    baseUrl: "https://chatgpt.com/backend-api",
  };
  const adapter = providers.get("openai-codex");
  assert.equal(adapter.streamSimple(model, context, options), "native-stream");
  adapter.streamSimple(model, context, options);
  const [routed, transcript, forwarded] = requests[0];
  assert.equal(routed.baseUrl, "http://127.0.0.1:8787/codex");
  assert.equal(model.baseUrl, "https://chatgpt.com/backend-api");
  assert.equal(transcript, context);
  assert.equal(forwarded.apiKey, options.apiKey);
  assert.equal(forwarded.headers.authorization, options.headers.authorization);
  assert.equal(forwarded.onPayload, options.onPayload);
  assert.equal(forwarded.transport, "sse");
  assert.equal(forwarded.headers["x-ren-pi-session-id"], session);
  const firstRun = forwarded.headers["x-ren-pi-turn-id"];
  assert.match(firstRun, /^[0-9a-f-]{36}$/);
  assert.equal(requests[1][2].headers["x-ren-pi-turn-id"], firstRun);
  assert.equal(options.headers["x-ren-pi-turn-id"], undefined);
  adapter.streamSimple(model, context, { ...options, sessionId: other });
  assert.equal(requests.at(-1)[2].headers["x-ren-pi-turn-id"], null);
  emit("agent_end");
  adapter.streamSimple(model, context, options);
  assert.equal(requests.at(-1)[2].headers["x-ren-pi-turn-id"], null);
  emit("agent_start");
  adapter.streamSimple(model, context, options);
  assert.notEqual(requests.at(-1)[2].headers["x-ren-pi-turn-id"], firstRun);
  emit("session_switch", other);
  providers.get("anthropic").streamSimple({ provider: "anthropic" }, context);
  assert.equal(requests.at(-1)[0].baseUrl, "http://127.0.0.1:8787/anthropic");
  assert.equal(requests.at(-1)[2].headers["x-ren-pi-session-id"], other);
  assert.equal(requests.at(-1)[2].headers["x-ren-pi-turn-id"], null);
  for (const provider of providers.values()) {
    assert.equal(provider.models, undefined);
    assert.equal(provider.oauth, undefined);
    assert.equal(provider.apiKey, undefined);
  }
});

test("OpenAI retains ChatGPT serialization identity while redirecting only native HTTP requests", async () => {
  const providers = new Map();
  const requests = [];
  const native = { streamSimple: (...args) => requests.push(args) };
  configure(
    { on() {}, registerProvider: (id, adapter) => providers.set(id, adapter) },
    new URL("https://gateway.test"),
    {
      anthropicMessagesApi: () => native,
      openAICodexResponsesApi: () => native,
      openAIResponsesApi: () => native,
    },
  );
  const calls = [];
  const response = new Response("fixture");
  const fetch = (...args) => {
    calls.push(args);
    return response;
  };
  const options = {
    apiKey: "oauth-fixture",
    fetch,
    sessionId: session,
    maxTokens: 123,
    temperature: 0.5,
    cacheRetention: "long",
    onPayload() {},
  };
  const model = {
    provider: "openai",
    baseUrl: "https://gateway.test/openai/v1",
    compat: { supportsStrictMode: true },
  };
  const context = { messages: [] };
  providers.get("openai").streamSimple(model, context, options);
  const [nativeModel, transcript, forwarded] = requests[0];
  assert.equal(nativeModel.baseUrl, "https://api.openai.com/v1");
  assert.equal(model.baseUrl, "https://gateway.test/openai/v1");
  assert.equal(nativeModel.compat, model.compat);
  assert.equal(transcript, context);
  for (const key of [
    "apiKey",
    "maxTokens",
    "temperature",
    "cacheRetention",
    "onPayload",
  ])
    assert.equal(forwarded[key], options[key]);
  const controller = new AbortController();
  const init = {
    method: "POST",
    body: '{"native":true}',
    headers: { authorization: "Bearer fixture" },
    signal: controller.signal,
  };
  assert.equal(
    await forwarded.fetch(
      "https://api.openai.com/v1/responses?fixture=1",
      init,
    ),
    response,
  );
  assert.equal(
    calls[0][0],
    "https://gateway.test/openai/v1/responses?fixture=1",
  );
  assert.equal(calls[0][1], init);
  await forwarded.fetch(
    new Request("https://api.openai.com/v1/responses", init),
  );
  const request = calls[1][0];
  assert.equal(request.url, "https://gateway.test/openai/v1/responses");
  assert.equal(request.method, "POST");
  assert.equal(request.headers.get("authorization"), "Bearer fixture");
  assert.equal(await request.text(), init.body);
  controller.abort();
  assert.equal(request.signal.aborted, true);
  for (const url of [
    "https://other.test/v1/responses",
    "https://api.openai.com/v1/unknown",
    "https://secret@api.openai.com/v1/responses",
  ]) {
    assert.throws(
      () => forwarded.fetch(url, init),
      /^Error: Ren: unexpected OpenAI inference URL$/,
    );
  }
  assert.equal(calls.length, 2);
});

test("sends the user token only when one is given", () => {
  const providers = new Map();
  const requests = [];
  const implementation = { streamSimple: (...args) => requests.push(args) };
  const apis = {
    anthropicMessagesApi: () => implementation,
    openAICodexResponsesApi: () => implementation,
    openAIResponsesApi: () => implementation,
  };
  const pi = {
    on: () => {},
    registerProvider: (name, value) => providers.set(name, value),
  };
  const model = { provider: "anthropic", baseUrl: "https://api.anthropic.com" };
  configure(pi, new URL("http://127.0.0.1:8787"), apis, "renu_fixture");
  providers.get("anthropic").streamSimple(model, { messages: [] }, {});
  assert.equal(requests[0][2].headers["x-ren-user-token"], "renu_fixture");
  configure(pi, new URL("http://127.0.0.1:8787"), apis, undefined);
  providers.get("anthropic").streamSimple(model, { messages: [] }, {});
  assert.equal("x-ren-user-token" in requests[1][2].headers, false);
});
