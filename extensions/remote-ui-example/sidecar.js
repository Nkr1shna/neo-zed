#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { spawn } = require("node:child_process");

const CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL = "https://auth.openai.com/oauth/token";
const REDIRECT_URI = "http://localhost:1455/auth/callback";
const OAUTH_SCOPE = "openid profile email offline_access";
const ACCOUNT_ID_CLAIM = "https://api.openai.com/auth";
const USAGE_URL = "https://chatgpt.com/backend-api/wham/usage";
const STATE_PATH = path.join(process.cwd(), "codex-chatgpt-auth.json");
const SUCCESS_HTML =
  "<!doctype html><html><body><p>Authentication successful. Return to Zed.</p></body></html>";
const USAGE_CACHE_MS = 5 * 60 * 1000;

const runtime = {
  loginPromise: null,
  tokens: loadTokens(),
  lastSnapshot: null,
  lastFetchedAt: 0,
  lastError: null,
};

process.stdin.setEncoding("utf8");

let inputBuffer = "";
process.stdin.on("data", (chunk) => {
  inputBuffer += chunk;
  for (;;) {
    const newlineIndex = inputBuffer.indexOf("\n");
    if (newlineIndex === -1) {
      break;
    }

    const line = inputBuffer.slice(0, newlineIndex).trim();
    inputBuffer = inputBuffer.slice(newlineIndex + 1);
    if (!line) {
      continue;
    }

    handleLine(line).catch((error) => {
      console.error("[codex-sidecar] request handling failed:", error);
    });
  }
});

process.stdin.on("end", async () => {
  await shutdown();
});

async function handleLine(line) {
  let request;
  try {
    request = JSON.parse(line);
  } catch (error) {
    writeMessage({
      jsonrpc: "2.0",
      id: null,
      error: { code: -32700, message: `invalid JSON: ${error.message}` },
    });
    return;
  }

  const { id, method, params } = request;
  if (request.jsonrpc !== "2.0" || typeof method !== "string") {
    writeMessage({
      jsonrpc: "2.0",
      id: id ?? null,
      error: { code: -32600, message: "invalid JSON-RPC request" },
    });
    return;
  }

  try {
    const result = await handleMethod(method, params);
    writeMessage({ jsonrpc: "2.0", id, result });
  } catch (error) {
    writeMessage({
      jsonrpc: "2.0",
      id,
      error: {
        code: -32000,
        message: error instanceof Error ? error.message : String(error),
      },
    });
  }
}

async function handleMethod(method, _params) {
  switch (method) {
    case "usage.snapshot":
      return await snapshotUsage(false);
    case "usage.refresh":
      return await snapshotUsage(true);
    case "auth.begin-login":
      return await beginLogin();
    case "auth.logout":
      return await logout();
    default:
      throw new Error(`unknown sidecar method \`${method}\``);
  }
}

function writeMessage(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function loadTokens() {
  try {
    const raw = fs.readFileSync(STATE_PATH, "utf8");
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") {
      return null;
    }

    return {
      accessToken: typeof parsed.access_token === "string" ? parsed.access_token : null,
      refreshToken: typeof parsed.refresh_token === "string" ? parsed.refresh_token : null,
      accountId: typeof parsed.account_id === "string" ? parsed.account_id : null,
      expiresAt: typeof parsed.expires_at === "number" ? parsed.expires_at : 0,
      lastRefreshAt: typeof parsed.last_refresh_at === "number" ? parsed.last_refresh_at : 0,
    };
  } catch {
    return null;
  }
}

function persistTokens(tokens) {
  fs.writeFileSync(
    STATE_PATH,
    JSON.stringify(
      {
        access_token: tokens.accessToken,
        refresh_token: tokens.refreshToken,
        account_id: tokens.accountId,
        expires_at: tokens.expiresAt,
        last_refresh_at: Date.now(),
      },
      null,
      2,
    ),
    "utf8",
  );
}

function clearPersistedTokens() {
  if (fs.existsSync(STATE_PATH)) {
    fs.unlinkSync(STATE_PATH);
  }
}

async function snapshotUsage(forceRefresh) {
  if (runtime.loginPromise) {
    return pendingSnapshot("Waiting for the ChatGPT OAuth callback.");
  }

  if (!runtime.tokens?.refreshToken) {
    return signedOutSnapshot();
  }

  if (
    !forceRefresh &&
    runtime.lastSnapshot &&
    Date.now() - runtime.lastFetchedAt < USAGE_CACHE_MS
  ) {
    return runtime.lastSnapshot;
  }

  const snapshot = await fetchUsageSnapshot(forceRefresh);
  runtime.lastSnapshot = snapshot;
  runtime.lastFetchedAt = Date.now();
  return snapshot;
}

async function beginLogin() {
  if (runtime.loginPromise) {
    return pendingSnapshot("Waiting for the ChatGPT OAuth callback.");
  }

  runtime.loginPromise = runLoginFlow()
    .then(async () => {
      runtime.lastSnapshot = await fetchUsageSnapshot(true);
      runtime.lastFetchedAt = Date.now();
      runtime.lastError = null;
    })
    .catch((error) => {
      runtime.lastError = error instanceof Error ? error.message : String(error);
      runtime.lastSnapshot = errorSnapshot(runtime.lastError);
      runtime.lastFetchedAt = Date.now();
    })
    .finally(() => {
      runtime.loginPromise = null;
    });

  return pendingSnapshot("Opened ChatGPT sign-in in your browser.");
}

async function logout() {
  runtime.tokens = null;
  runtime.lastSnapshot = signedOutSnapshot();
  runtime.lastFetchedAt = Date.now();
  runtime.lastError = null;
  clearPersistedTokens();
  return runtime.lastSnapshot;
}

async function runLoginFlow() {
  const verifier = base64Url(crypto.randomBytes(48));
  const challenge = base64Url(crypto.createHash("sha256").update(verifier).digest());
  const state = crypto.randomBytes(16).toString("hex");
  const authorizeUrl = new URL(AUTHORIZE_URL);
  authorizeUrl.searchParams.set("response_type", "code");
  authorizeUrl.searchParams.set("client_id", CLIENT_ID);
  authorizeUrl.searchParams.set("redirect_uri", REDIRECT_URI);
  authorizeUrl.searchParams.set("scope", OAUTH_SCOPE);
  authorizeUrl.searchParams.set("code_challenge", challenge);
  authorizeUrl.searchParams.set("code_challenge_method", "S256");
  authorizeUrl.searchParams.set("state", state);
  authorizeUrl.searchParams.set("id_token_add_organizations", "true");
  authorizeUrl.searchParams.set("codex_cli_simplified_flow", "true");
  authorizeUrl.searchParams.set("originator", "pi");

  const oauthServer = await startOAuthServer(state);
  openBrowser(authorizeUrl.toString());

  try {
    const code = await oauthServer.waitForCode();
    if (!code) {
      throw new Error("ChatGPT sign-in timed out before the OAuth callback arrived");
    }

    const tokenResponse = await exchangeAuthorizationCode(code, verifier);
    runtime.tokens = tokenResponse;
    persistTokens(tokenResponse);
  } finally {
    oauthServer.close();
  }
}

function startOAuthServer(expectedState) {
  let receivedCode = null;
  let waiters = [];
  const server = http.createServer((request, response) => {
    try {
      const url = new URL(request.url || "", "http://localhost");
      if (url.pathname !== "/auth/callback") {
        response.statusCode = 404;
        response.end("Not found");
        return;
      }

      if (url.searchParams.get("state") !== expectedState) {
        response.statusCode = 400;
        response.end("State mismatch");
        return;
      }

      const code = url.searchParams.get("code");
      if (!code) {
        response.statusCode = 400;
        response.end("Missing authorization code");
        return;
      }

      receivedCode = code;
      response.statusCode = 200;
      response.setHeader("Content-Type", "text/html; charset=utf-8");
      response.end(SUCCESS_HTML);
      waiters.forEach((resolve) => resolve(code));
      waiters = [];
    } catch (error) {
      response.statusCode = 500;
      response.end(error instanceof Error ? error.message : "Internal error");
    }
  });

  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(1455, "127.0.0.1", () => {
      server.removeListener("error", reject);
      resolve({
        close() {
          server.close();
        },
        waitForCode() {
          if (receivedCode) {
            return Promise.resolve(receivedCode);
          }

          return new Promise((resolveWait) => {
            const timeout = setTimeout(() => resolveWait(null), 10 * 60 * 1000);
            waiters.push((code) => {
              clearTimeout(timeout);
              resolveWait(code);
            });
          });
        },
      });
    });
  });
}

async function exchangeAuthorizationCode(code, verifier) {
  const response = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      client_id: CLIENT_ID,
      code,
      code_verifier: verifier,
      redirect_uri: REDIRECT_URI,
    }),
  });

  if (!response.ok) {
    const responseBody = await response.text().catch(() => "");
    throw new Error(
      `authorization code exchange failed with HTTP ${response.status}${
        responseBody ? `: ${responseBody}` : ""
      }`,
    );
  }

  const json = await response.json();
  return normalizeTokens(json);
}

async function refreshAccessToken() {
  if (!runtime.tokens?.refreshToken) {
    throw new Error("no refresh token is available");
  }

  const response = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "refresh_token",
      client_id: CLIENT_ID,
      refresh_token: runtime.tokens.refreshToken,
    }),
  });

  if (!response.ok) {
    const responseBody = await response.text().catch(() => "");
    throw new Error(
      `token refresh failed with HTTP ${response.status}${
        responseBody ? `: ${responseBody}` : ""
      }`,
    );
  }

  const json = await response.json();
  const tokens = normalizeTokens({
    ...json,
    refresh_token: json.refresh_token || runtime.tokens.refreshToken,
  });
  runtime.tokens = tokens;
  persistTokens(tokens);
  return tokens;
}

function normalizeTokens(json) {
  if (!json || typeof json !== "object") {
    throw new Error("token response was not an object");
  }

  if (typeof json.access_token !== "string" || typeof json.refresh_token !== "string") {
    throw new Error("token response is missing access or refresh tokens");
  }

  const expiresIn = typeof json.expires_in === "number" ? json.expires_in : 3600;
  const accountId =
    extractAccountId(json.access_token) || extractAccountId(json.id_token) || null;

  return {
    accessToken: json.access_token,
    refreshToken: json.refresh_token,
    accountId,
    expiresAt: Date.now() + expiresIn * 1000,
    lastRefreshAt: Date.now(),
  };
}

async function ensureAccessToken(forceRefresh) {
  if (!runtime.tokens?.refreshToken) {
    return null;
  }

  const expiresSoon = !runtime.tokens.expiresAt || runtime.tokens.expiresAt < Date.now() + 5 * 60 * 1000;
  if (forceRefresh || expiresSoon) {
    await refreshAccessToken();
  }

  return runtime.tokens.accessToken;
}

async function fetchUsageSnapshot(forceRefresh) {
  const accessToken = await ensureAccessToken(forceRefresh);
  if (!accessToken) {
    return signedOutSnapshot();
  }

  let response = await fetchUsage(accessToken);
  if (response.status === 401 || response.status === 403) {
    await refreshAccessToken();
    response = await fetchUsage(runtime.tokens.accessToken);
  }

  if (!response.ok) {
    throw new Error(`usage request failed with HTTP ${response.status}`);
  }

  const json = await response.json();
  return normalizeUsageSnapshot(json);
}

async function fetchUsage(accessToken) {
  const headers = {
    Accept: "application/json",
    Authorization: `Bearer ${accessToken}`,
  };

  if (runtime.tokens?.accountId) {
    headers["ChatGPT-Account-Id"] = runtime.tokens.accountId;
  }

  return fetch(USAGE_URL, { headers });
}

function normalizeUsageSnapshot(json) {
  const rateLimit = asObject(json.rate_limit);
  const primaryWindow = asObject(rateLimit.primary_window);
  const secondaryWindow = asObject(rateLimit.secondary_window);
  const planType = typeof json.plan_type === "string" ? json.plan_type : null;
  const primaryUsedPercent = readPercent(primaryWindow.used_percent);
  const secondaryUsedPercent = readPercent(secondaryWindow.used_percent);
  const credits = asObject(json.credits);

  let detail = `${primaryUsedPercent}% used in the 5 hour Codex window`;
  if (typeof secondaryUsedPercent === "number") {
    detail += `, ${secondaryUsedPercent}% used in the 7 day window`;
  }
  if (credits.has_credits === true && credits.unlimited !== true && typeof credits.balance === "number") {
    detail += `, $${credits.balance.toFixed(2)} credits left`;
  }

  return {
    auth_status: "authenticated",
    status_label: "ChatGPT connected",
    detail,
    plan_type: planType,
    account_label: runtime.tokens?.accountId
      ? `Account ${runtime.tokens.accountId.slice(-8)}`
      : null,
    primary_window_label: "5h Codex window",
    secondary_window_label: "7d Codex window",
    primary_used_percent: primaryUsedPercent,
    secondary_used_percent: secondaryUsedPercent,
    busy: false,
  };
}

function signedOutSnapshot() {
  return {
    auth_status: "signed-out",
    status_label: "Sign in to ChatGPT",
    detail: "The sidecar will open a browser and store the refresh token in the extension work directory.",
    plan_type: null,
    account_label: null,
    primary_window_label: "5h Codex window",
    secondary_window_label: "7d Codex window",
    primary_used_percent: 0,
    secondary_used_percent: 0,
    busy: false,
  };
}

function pendingSnapshot(detail) {
  return {
    auth_status: "pending",
    status_label: "Waiting for ChatGPT sign-in",
    detail,
    plan_type: null,
    account_label: null,
    primary_window_label: "5h Codex window",
    secondary_window_label: "7d Codex window",
    primary_used_percent: 0,
    secondary_used_percent: 0,
    busy: true,
  };
}

function errorSnapshot(errorMessage) {
  return {
    auth_status: "error",
    status_label: "ChatGPT sign-in failed",
    detail: errorMessage,
    plan_type: null,
    account_label: null,
    primary_window_label: "5h Codex window",
    secondary_window_label: "7d Codex window",
    primary_used_percent: 0,
    secondary_used_percent: 0,
    busy: false,
  };
}

function extractAccountId(token) {
  if (typeof token !== "string") {
    return null;
  }

  try {
    const parts = token.split(".");
    if (parts.length !== 3) {
      return null;
    }
    const payload = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
    return payload?.[ACCOUNT_ID_CLAIM]?.chatgpt_account_id || null;
  } catch {
    return null;
  }
}

function readPercent(value) {
  if (typeof value !== "number" || Number.isNaN(value)) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round(value)));
}

function asObject(value) {
  return value && typeof value === "object" ? value : {};
}

function base64Url(buffer) {
  return buffer
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function openBrowser(url) {
  let command;
  let args;
  if (process.platform === "darwin") {
    command = "open";
    args = [url];
  } else if (process.platform === "win32") {
    command = "cmd";
    args = ["/c", "start", "", url];
  } else {
    command = "xdg-open";
    args = [url];
  }

  const child = spawn(command, args, {
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}

async function shutdown() {
  if (runtime.loginPromise) {
    try {
      await runtime.loginPromise;
    } catch {
      // Ignore login errors while shutting down the process.
    }
  }
  process.exit(0);
}
