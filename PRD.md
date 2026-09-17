# ApeMe — apeme.fun

**The first terminal built for stonk tokens. Ape memes, ape stonks, ape whatever is running.**

Native iOS app. Solana first, Robinhood Chain second. No smart contracts, no token of our own, no launchpad of our own.

---

## 1. What is happening on-chain

Two kinds of things now exist on Solana:

1. **Stock tokens.** Real shares turned into tokens by three issuers: xStocks (Backed), Backpack Securities, and PreStocks for pre-IPO names. NVDAx is Nvidia. BULL is Webull. SPACEX is SpaceX. About 470 mints today.
2. **Memes priced in stock tokens.** Launchpads now let anyone launch a meme coin whose quote asset is a stock token instead of SOL. A cat coin priced in NVDAx. Three launchpads do this on Solana: StonkFun (on Raydium LaunchLab), pump.fun Custom Pairs (since 9 Sep 2026, 93 stock pairs), and Clawpump (on Meteora DBC). On Robinhood Chain: Pons.

This is the "stonk meta". STONK, the StonkFun token, sits around $180M market cap. pump.fun joined the meta a week ago. There is no good place to watch or trade it.

## 2. What ApeMe is

One app that shows every stock token on Solana, every meme paired with it from every launchpad, and lets you buy any of it with one tap.

Three screens in v1:

1. **Stocks.** Every stock token, price, change, how many memes are paired with it, volume today. NYSE open or closed. A Pre-IPO section for SpaceX, OpenAI, Anthropic and friends.
2. **Memes for that stock.** Tap Nvidia, see every meme priced in NVDAx across StonkFun, pump.fun and Meteora DBC. Market cap, 24h change, launchpad badge, graduation progress, holder rewards and tax if any.
3. **Token page.** Live chart, live trade tape, holders, and an Ape button. One tap, preset amount, no popup.

After v1, in the same app:

- **KOL calls.** We track a list of X accounts. When one posts about a token we index, everyone watching that account gets a push with an Ape button, and the call is pinned on the chart at the price it was posted. No X login needed for users.
- **Callouts feed.** Every call with the KOL's avatar, entry price, current return. Receipts, not vibes.
- **Mixes.** Buy a basket of memes in one tap. One swap per leg, no contract.
- **Auto-ape.** Pro users can let a call from a chosen KOL execute automatically with a cap.

## 3. Why us

- **Stock-aware on every card.** Nobody else shows "NYSE closed, this meme is priced in TSLAx, TSLA closed at $412". dex.fun is a general web terminal. StonkFun and pump.fun each show only their own launches.
- **All launchpads in one list.** StonkFun, pump.fun, Meteora DBC, and later Pons on Robinhood Chain. One card format.
- **Ape the call, not the tweet.** The push is the buy button. Two seconds from lock screen to confirmed.
- **Native iOS.** dex.fun has mobile on its roadmap. We are there first.
- **KOLs get paid without signing up.** Fees from apes on a KOL's call accrue to their X handle. They claim later with Sign in with X. Precedent: Bags.fm, Believe, dex.fun's own verify-to-claim escrow.

## 4. How we make money

- **1% fee on every ape**, set on the Jupiter swap. Nothing custodial.
- **0.3% of that goes to the KOL** whose call was tapped. 0.5% on a mix parsed from their post. Held by X handle until claimed. Unclaimed after 90 days rolls to us.
- **Later:** Pro tier for auto-ape and alerts. KOLs paying to pin a call. Never tokens paying for placement.

At $1M daily ape volume that is $7,000 a day to us after KOL share.

## 5. How it is built

- **Phone:** Swift, SwiftUI. Privy SDK for login and an embedded Solana wallet that signs on the device. Keys never leave the phone.
- **Backend:** Cloudflare Workers with Hono. KV for hot numbers, D1 for the record, a Durable Object per token for the live WebSocket feed, a Queue for pushes to APNs.
- **Indexer:** One Node microservice on a VPS holding a Yellowstone gRPC stream (constant-k Nexus) for six programs: pump.fun, PumpSwap, Raydium LaunchLab, Raydium CPMM, Meteora DBC, Meteora DAMM v2. Decodes with Shyft's open-source parser, posts batches to the Worker. Trades reach the phone in about 300ms.
- **Catalog:** Public APIs, no keys: StonkFun (stock list, launches), Raydium LaunchLab (launches, trades, klines), pump.fun (coins with quote mint), GeckoTerminal and DexScreener (after graduation), Backpack (market hours), PreStocks (valuations), Finnhub (NYSE close).
- **Swaps:** Jupiter, always. It routes across every DEX and takes our fee parameter.
- **Never:** smart contracts, our own token, a launchpad, holding user funds or shares, minting or redeeming stocks.

Costs: Cloudflare $5, X reads about $150 for 50 KOLs, constant-k free for a month then $49 to $179. Everything else is free tier.

## 6. Launch plan

- **Gated.** First 100 users, then 200, then 500. TestFlight first.
- **Stocklana hackathon** checkpoint Friday 18 Sep 2026, Consumer track. Side bounties: PreStocks (Pre-IPO section) and Meteora DBC (DBC pool monitoring).
- **Week 1:** three screens, indexer, one-tap ape. **Week 2:** KOL calls and pushes, callouts feed, Robinhood Chain lane. **Week 3:** mixes, auto-ape, public unclaimed board for KOLs, gate opens to 200.

## 7. What it is not

Not a launchpad. Not a wallet. Not a broker. Not a web app. Not a token. It is the terminal you open when the stonk meta is moving.
