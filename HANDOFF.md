# ApeMe handoff (read this first on a new machine)

Written 2026-09-18 on the VPS session. Everything here is verified state, not plan.

## What ApeMe is
pump.fun-style floor for tokenized stocks on Solana. Stocks (xStocks NVDAx, SPYx…, Backpack, PreStocks, Tessera) are the *quote* currency; memes launched against them on StonkFun (Raydium LaunchLab/CPMM), pump.fun (+PumpSwap) and Meteora DBC (+DAMM v2) are what people ape. Native Swift iOS app, no smart contracts. Stocklana hackathon, submission **Fri 25 Sep 2026, 4pm ET**. GitHub org `ape-me`, user is `iam-joey` (git identity iam-joey / darushyam143@gmail.com).

## Rules from Joey
- **No AI attribution anywhere.** No `Co-Authored-By: Claude`, no "Generated with Claude" in commits, PRs or files. All work under his profile.
- Short, concise, visual. He reviews artifacts (claude.ai artifacts, dark theme). Don't over-explain. One new artifact per topic, don't re-use an old one for a new topic.
- Native Swift, iOS only for v1. Never pitch web or React Native.
- Confirm before infra/deploy actions that are new; pushing commits he asked for is fine without asking.
- Dark UI for this project (his only dark-mode project).

## The three services
| repo | where it runs | state |
|---|---|---|
| `ape-me/ape-indexer` (Rust) | VPS, systemd `ape-indexer` | live, hardened, soak-tested |
| `ape-me/apme-be` (Hono, Cloudflare Workers) | https://apme-be.iamjoey.workers.dev | live |
| `ape-me/ape-fe` (Swift) | Mac / Xcode | **not started** — this is the next job |

Production later: a dedicated VPS just for indexer + Postgres. Current VPS is shared/overloaded (explains latency jitter), nothing in code depends on it.

## Backend API the app uses (all JSON, all live now)
```
GET /health                                   db + indexer lag
GET /v1/stocks                                {stocks:[Stock], asOf}
GET /v1/stocks/:mint/tokens?sort=volume|new|mcap&limit=50&cursor=   {stock, tokens:[TokenCard], next}
GET /v1/tokens/:mint                          TokenHeader (TokenCard + creator/decimals/supply/pools/uri + stock{})
GET /v1/tokens/:mint/candles?tf=1m|5m|15m|1h|4h|1d&limit=300&before=   {mint, tf, candles:[{t,o,h,l,c,v,n}]}
GET /v1/tokens/:mint/trades?limit=100&before=   {mint, trades:[Trade]}
WS  /ws/floor        every trade on every token, as JSON arrays of WsTrade
WS  /ws/:mint        trades for one token. Send "ping" every 25s, get "pong".
```
Exact shapes (zod) are in `apme-be/src/contract.ts` — generate the Swift models from it. Prices: `priceQuote` is in units of the stock (e.g. NVDAx per meme), `priceUsd = priceQuote × stock.priceUsd`. `mcapUsd` = priceUsd × supply. Trade `base`/`quote` are wallet-side amounts (transfer tax included), matching explorers. Errors: `{error, requestId}` with 400/404/429/500. Rate limit 600 req/min per IP. Cache 2–5s on reads.

Real ids to test with: NVDAx `Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh`; busiest meme on it NIU `GXL9wD1F5TzVXfZ33fKkBxeuNi7wKMUR25SDEBGEZ9mN`.

## Timeline that's left
- Thu 18 (today): phase 0 closes (second 1h soak vs Raydium running; run 1 found and fixed a block-time bucketing bug).
- Fri–Sun: **ape-fe** three screens: Stocks list → Stock floor (live cards from /ws/floor) → Token page (candles + live tape from /ws/:mint). Read-only, no wallet yet.
- Mon: Privy embedded wallet + one-tap ape via Jupiter (Jupiter referral bps = our fee). 
- Tue: Meteora DBC bounty angle ($5k). Also PreStocks bounty ($10k, needs a Pre-IPO section; hide Tessera quotes in-app to stay eligible; ask PreStocks on X whether xStocks SPCXx counts).
- Wed–Fri: TestFlight, demo video, submit with a day of buffer.

Joey still owes: paid Apple Developer account (by Sun), `ape-me/ape-fe` repo, apeme.fun added to the Cloudflare account (yerradarwin acct), dedicated VPS (later).

## Design language for the app
Dark. Display face Bricolage Grotesque, body IBM Plex Sans, mono IBM Plex Mono (used in all artifacts so far; pick SF Pro equivalents on iOS if custom fonts are a hassle). Palette: bg #0b0e12, surface #12161c, line #232a33, ink #eef2f5, muted #a7b1bc, green #5fe39a, amber #f5b640, blue #6db3ff, red #ff5c5c. Feel = dex.fun / pump.fun terminal: dense cards, live numbers, big Ape button.

## Things learned the hard way (don't re-learn)
- Solana block_time lags wall clock 1–4s; our pipeline adds ~1s. Trades reach a phone ~1s after confirmation.
- Raydium's own 1m klines occasionally miss the last trades of a minute; when we disagree, check the chain, not Raydium.
- Chart history for older tokens is backfilled from Raydium/pump.fun candles; DBC tokens have none (parked, "show live charts, who cares about old data").
- Vybe / DexScreener / Gecko can't give history for these pairs.
- X/KOL engine later: twitterapi.io filter rules, watch tokens not accounts.

## VPS-only details (not needed on the Mac)
Indexer env `/root/ape-indexer/.env` (Kaldera gRPC, RPC, DB, INGEST_URL/SECRET). Postgres docker `apeme-pg`, read-only user `apeme_ro`. Worker reads via Hyperdrive → Cloudflare Tunnel `apeme-pg` (`pg.sendsol.lol`) → Access service token. Deploy Worker: `cd /root/apme-be && source /root/.cf_token && bunx wrangler deploy`. Watchdog cron restarts the indexer if it stalls or falls >750 slots behind. Soak: `/root/ape-indexer/soak/soak.sh`.
