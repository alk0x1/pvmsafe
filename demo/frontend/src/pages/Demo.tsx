import { useEffect, useMemo, useState } from "react";
import { Highlight, themes } from "prism-react-renderer";
import {
  AccountsResponse,
  call,
  Example,
  formatAddr,
  formatBalance,
  getAccounts,
  getState,
  Kind,
  parseBalance,
  Sender,
  setup,
  StateEntry,
  StateResponse,
} from "../api";

type Addrs = { vulnerable: string; safe: string } | null;

type ActivityEntry = {
  id: number;
  succeeded: boolean;
  headline: string;
  detail: string;
  timestamp: number;
};

type ActionForm =
  | {
      kind: "transfer";
      to: "alice" | "bob";
      amount: string;
    }
  | {
      kind: "deposit";
      amount: string;
    }
  | {
      kind: "withdraw";
      amount: string;
    };

type ExampleMeta = {
  id: Example;
  appName: string;
  tagline: string;
  feature: string;
  description: string;
  liveChain: boolean;
  balanceLabel: string;
  ledgerLabels: string[];
  walletLabels: { alice: string; bob: string };
  initialForms: ActionForm[];
  snippet: {
    vuln: string;
    safe: string;
    vulnNote: string;
    safeNote: string;
    compileError: string;
    compileErrorNote: string;
  };
  isCompromised: (entries: StateEntry[], accounts: AccountsResponse | null) => boolean;
};

const EXAMPLES: ExampleMeta[] = [
  {
    id: "erc20",
    appName: "PvmToken",
    tagline: "an ERC-20 on pallet-revive",
    feature: "arithmetic safety via refinement types",
    description:
      "The usual token. transfer / balanceOf / totalSupply. One of these two deployments has no underflow guard on the sender's balance.",
    liveChain: true,
    balanceLabel: "balance",
    ledgerLabels: ["total supply"],
    walletLabels: { alice: "Alice", bob: "Bob" },
    initialForms: [{ kind: "transfer", to: "alice", amount: "1" }],
    snippet: {
      vuln: `let sender_balance = load(&balance_key(&caller));

let new_sender = sender_balance - amount;
//  ⚠ no guard — if balance < amount,
//     the subtraction underflows to 2²⁵⁶−1

store(&balance_key(&caller), new_sender);`,
      safe: `let sender_balance = load(&balance_key(&caller));

if sender_balance < amount {
    return Err(InsufficientBalance);
}

let new_sender = sender_balance - amount;
//  ✓ proved by pvmsafe: the early-return guard
//    threads \`sender_balance >= amount\` into Γ

store(&balance_key(&caller), new_sender);`,
      vulnNote:
        "compiles fine. at runtime Bob's 0 minus 1 wraps to 2²⁵⁶−1. funds stolen.",
      safeNote:
        "pvmsafe rejects at compile time unless the guard is present and the refinement discharges.",
      compileError: `error: pvmsafe: subtraction \`sender_balance - amount\` may underflow; not provable from caller's assumptions
  --> pvmsafe-example/src/pvmsafe-example-erc20.rs:54:26
   |
54 |         let new_sender = sender_balance - amount;
   |                          ^^^^^^^^^^^^^^^^^^^^^^^`,
      compileErrorNote:
        "$ cargo build --release  ·  if you'd added pvmsafe to the vulnerable version, this is what you'd see",
    },
    isCompromised: (entries) => {
      const bob = entries.find((e) => e.role === "attacker");
      const supply = entries.find((e) => e.label === "total supply");
      if (!bob || !supply) return false;
      return parseBalance(bob.value) > parseBalance(supply.value) * 1_000n;
    },
  },
  {
    id: "vault",
    appName: "PvmVault",
    tagline: "a share-based vault on pallet-revive",
    feature: "conservation-of-value invariants",
    description:
      "A simple vault. Alice deposits assets and gets shares; withdraw burns shares and returns assets. One deployment forgets to decrement the caller's share slot.",
    liveChain: true,
    balanceLabel: "your shares",
    ledgerLabels: ["total assets", "total shares"],
    walletLabels: { alice: "Alice", bob: "Bob" },
    initialForms: [
      { kind: "withdraw", amount: "1000" },
      { kind: "deposit", amount: "100" },
    ],
    snippet: {
      vuln: `fn withdraw(shares: U256) -> Result<(), Error> {
    let supply = total_shares();
    let assets = total_assets();
    let payout = shares * assets / supply;

    store(&total_shares_key(), supply - shares);
    store(&total_assets_key(), assets - payout);
    //  ⚠ no balance check, no user slot update
    //     caller receives payout they never earned
}`,
      safe: `#[pvmsafe::invariant(conserves(shares))]
mod vault {
    fn withdraw(
        #[pvmsafe::refine(shares > 0)] shares: U256,
    ) -> Result<(), Error> {
        if user_shares < shares {
            return Err(InsufficientShares);
        }
        ...
        #[pvmsafe::delta(shares = -shares)]
        store(&shares_key(&caller), user_shares - shares);

        #[pvmsafe::delta(shares = shares)]
        store(&total_shares_key(), supply - shares);
        //  ✓ delta sum = 0, conservation proved
    }
}`,
      vulnNote:
        "compiles fine. at runtime Bob's withdraw drains the vault without ever touching his share slot.",
      safeNote:
        "pvmsafe refuses to compile if any write is missing a matching delta annotation.",
      compileError: `error: pvmsafe: conservation invariant violated in \`withdraw\` for group \`shares\`; declared deltas do not sum to zero on this path
  --> pvmsafe-example/src/pvmsafe-example-vault.rs:88:5
   |
88 |     fn withdraw(shares: U256) -> Result<(), Error> {
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^`,
      compileErrorNote:
        "$ cargo build --release  ·  with `#[pvmsafe::invariant(conserves(shares))]` declared, the missing write is exposed",
    },
    isCompromised: (entries) => {
      const assets = entries.find((e) => e.label === "total assets");
      const aliceShares = entries.find((e) => e.role === "victim");
      if (!assets || !aliceShares) return false;
      return (
        parseBalance(assets.value) === 0n &&
        parseBalance(aliceShares.value) > 0n
      );
    },
  },
  {
    id: "effects",
    appName: "PvmPool",
    tagline: "a withdrawable deposit pool on pallet-revive",
    feature: "CEI ordering via effect types",
    description:
      "A pool where users can deposit and withdraw. withdraw() transfers value to the caller — the classical reentrancy shape. One implementation updates balance BEFORE the transfer; the other, AFTER. Only the first is sound.",
    liveChain: false,
    balanceLabel: "",
    ledgerLabels: [],
    walletLabels: { alice: "Alice", bob: "Bob" },
    initialForms: [],
    snippet: {
      vuln: `fn withdraw(amount: U256) -> Result<(), Error> {
    let bal = load(&balance_key(&caller));
    if bal < amount {
        return Err(Insufficient);
    }

    send_value(&caller, amount);
    //  ⚠ external call, then state update

    store(&balance_key(&caller), bal - amount);
    //  ⚠ a malicious caller re-enters withdraw()
    //     before this line runs and drains the pool
}`,
      safe: `#[pvmsafe::contract]
mod pool {
    #[pvmsafe::effect(read)]
    fn load(k: &[u8; 32]) -> U256 { /* ... */ }

    #[pvmsafe::effect(write)]
    fn store(k: &[u8; 32], v: U256) { /* ... */ }

    #[pvmsafe::effect(call)]
    fn send_value(to: &[u8; 20], amt: U256) { /* ... */ }

    pub fn withdraw(
        #[pvmsafe::refine(amount > 0)] amount: U256,
    ) -> Result<(), Error> {
        let bal = load(&balance_key(&caller));
        if bal < amount { return Err(Insufficient); }

        store(&balance_key(&caller), bal - amount);
        //  ✓ state update BEFORE external call
        send_value(&caller, amount);
    }
}`,
      vulnNote:
        "compiles fine. a malicious contract re-enters withdraw during send_value and drains the pool.",
      safeNote:
        "pvmsafe infers effects via call-graph fixpoint and rejects any write (or emit) that follows a call.",
      compileError: `error: pvmsafe: state write after external call; reentrancy risk
  --> pvmsafe-example/src/pvmsafe-example-pool.rs:18:9
   |
18 |         store(&balance_key(&caller), bal - amount);
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

error: note: earlier external call here
  --> pvmsafe-example/src/pvmsafe-example-pool.rs:15:9
   |
15 |         send_value(&caller, amount);
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^`,
      compileErrorNote:
        "$ cargo build --release  ·  the effect walker traces each path and rejects write-after-call",
    },
    isCompromised: () => false,
  },
];

function useLiveState(
  example: Example,
  contract: string | null,
  kind: Kind,
  pulse: number,
) {
  const [state, setState] = useState<StateResponse | null>(null);
  useEffect(() => {
    setState(null);
    if (!contract) return;
    let alive = true;
    const tick = async () => {
      try {
        const s = await getState(example, contract, kind);
        if (alive) setState(s);
      } catch {
        /* transient */
      }
    };
    tick();
    const id = setInterval(tick, 1500);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [example, contract, kind, pulse]);
  return state;
}

function WalletBadge({
  name,
  short,
  active,
  onClick,
}: {
  name: string;
  short: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button className={`wallet-badge ${active ? "active" : ""}`} onClick={onClick}>
      <span className={`wallet-avatar ${name.toLowerCase()}`}>{name[0]}</span>
      <span className="wallet-name">{name}</span>
      <span className="wallet-short">{short}</span>
    </button>
  );
}

function Instance({
  meta,
  kind,
  contract,
  accounts,
  connected,
  activity,
  stateRefresh,
}: {
  meta: ExampleMeta;
  kind: Kind;
  contract: string | null;
  accounts: AccountsResponse | null;
  connected: Sender;
  activity: ActivityEntry[];
  stateRefresh: number;
}) {
  const state = useLiveState(meta.id, contract, kind, stateRefresh);
  const accent: "danger" | "safe" = kind === "vulnerable" ? "danger" : "safe";
  const compromised =
    accent === "danger" && state != null && meta.isCompromised(state.entries, accounts);

  const entries = state?.entries ?? [];
  const ledgerEntries = entries.filter((e) => e.role === "ledger");
  const alice = entries.find((e) => e.role === "victim");
  const bob = entries.find((e) => e.role === "attacker");
  const you = connected === "alice" ? alice : bob;

  return (
    <section className={`instance ${accent} ${compromised ? "compromised" : ""}`}>
      <header className="instance-head">
        <div>
          <div className={`tag tag-${accent}`}>
            {accent === "danger" ? "✗ no pvmsafe" : "✓ pvmsafe"}
          </div>
          <div className="instance-title">
            {meta.appName}
            <span className={`instance-mark instance-mark-${accent}`}>
              {kind}
            </span>
          </div>
        </div>
        <code className="instance-addr" title={contract ?? ""}>
          {formatAddr(contract)}
        </code>
      </header>

      {!contract ? (
        <div className="instance-empty">
          <p>contract not deployed</p>
        </div>
      ) : (
        <>
          <div className="balance-card">
            <div className="balance-label">your {meta.balanceLabel}</div>
            <div className="balance-value">
              {you ? formatBalance(you.value) : "—"}
            </div>
            <div className="balance-sub">
              connected as <strong>{connected === "alice" ? "Alice" : "Bob"}</strong>
            </div>
          </div>

          <div className="info-row">
            {ledgerEntries.map((e) => (
              <div key={e.label} className="info-cell">
                <span className="info-label">{e.label}</span>
                <span className="info-value">{formatBalance(e.value)}</span>
              </div>
            ))}
          </div>

          <div className="wallets">
            <div className="wallet-line">
              <span className="wallet-dot wallet-dot-alice" />
              <span className="wallet-line-name">Alice</span>
              <span className="wallet-line-value">
                {alice ? formatBalance(alice.value) : "—"}
              </span>
            </div>
            <div className="wallet-line">
              <span className="wallet-dot wallet-dot-bob" />
              <span className="wallet-line-name">Bob</span>
              <span className="wallet-line-value">
                {bob ? formatBalance(bob.value) : "—"}
              </span>
            </div>
          </div>

          {compromised && (
            <div className="instance-banner">
              <span className="banner-icon">⚠</span>
              <strong>
                {meta.id === "erc20" ? "exploit executed" : "vault drained"}
              </strong>
            </div>
          )}

          <div className="activity">
            <div className="activity-head">activity</div>
            {activity.length === 0 ? (
              <div className="activity-empty">no transactions yet</div>
            ) : (
              <ul>
                {activity.slice(0, 5).map((a) => (
                  <li
                    key={a.id}
                    className={`activity-item ${
                      a.succeeded ? "tx-ok" : "tx-fail"
                    }`}
                  >
                    <span className="activity-icon">
                      {a.succeeded ? "✓" : "✗"}
                    </span>
                    <div className="activity-body">
                      <div className="activity-headline">{a.headline}</div>
                      <div className="activity-detail">{a.detail}</div>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}
    </section>
  );
}

function TransferForm({
  form,
  update,
  submit,
  busy,
  accounts,
}: {
  form: Extract<ActionForm, { kind: "transfer" }>;
  update: (next: ActionForm) => void;
  submit: () => void;
  busy: boolean;
  accounts: AccountsResponse | null;
}) {
  return (
    <div className="action-form">
      <div className="action-title">transfer</div>
      <div className="action-row">
        <label className="field">
          <span>to</span>
          <div className="recipient-picker">
            <button
              type="button"
              className={form.to === "alice" ? "active" : ""}
              onClick={() => update({ ...form, to: "alice" })}
            >
              Alice
            </button>
            <button
              type="button"
              className={form.to === "bob" ? "active" : ""}
              onClick={() => update({ ...form, to: "bob" })}
            >
              Bob
            </button>
          </div>
          <code className="field-hint">
            {accounts ? formatAddr(accounts[form.to]) : "…"}
          </code>
        </label>
        <label className="field">
          <span>amount</span>
          <input
            type="text"
            inputMode="numeric"
            value={form.amount}
            onChange={(e) => update({ ...form, amount: e.target.value })}
          />
        </label>
        <button className="send-btn" onClick={submit} disabled={busy}>
          {busy ? <span className="spinner" /> : "send"}
        </button>
      </div>
    </div>
  );
}

function AmountForm({
  form,
  update,
  submit,
  busy,
  title,
  verb,
}: {
  form: Extract<ActionForm, { kind: "deposit" | "withdraw" }>;
  update: (next: ActionForm) => void;
  submit: () => void;
  busy: boolean;
  title: string;
  verb: string;
}) {
  return (
    <div className="action-form">
      <div className="action-title">{title}</div>
      <div className="action-row">
        <label className="field">
          <span>amount</span>
          <input
            type="text"
            inputMode="numeric"
            value={form.amount}
            onChange={(e) => update({ ...form, amount: e.target.value })}
          />
        </label>
        <button className="send-btn" onClick={submit} disabled={busy}>
          {busy ? <span className="spinner" /> : verb}
        </button>
      </div>
    </div>
  );
}

function Snippet({
  snippet,
}: {
  snippet: ExampleMeta["snippet"];
}) {
  return (
    <div className="snippet">
      <h3>what pvmsafe prevents</h3>
      <p className="snippet-sub">
        the only difference between the two deployments
      </p>
      <div className="snippet-grid">
        <div className="snippet-col snippet-vuln">
          <div className="snippet-col-head">
            <span className="tag tag-danger">✗ vulnerable</span>
            <span className="snippet-note">{snippet.vulnNote}</span>
          </div>
          <Highlight theme={themes.vsDark} code={snippet.vuln} language="rust">
            {({ className, style, tokens, getLineProps, getTokenProps }) => (
              <pre className={className} style={style}>
                {tokens.map((line, i) => (
                  <div {...getLineProps({ line })} key={i}>
                    {line.map((token, key) => (
                      <span {...getTokenProps({ token })} key={key} />
                    ))}
                  </div>
                ))}
              </pre>
            )}
          </Highlight>
        </div>
        <div className="snippet-col snippet-safe">
          <div className="snippet-col-head">
            <span className="tag tag-safe">✓ safe</span>
            <span className="snippet-note">{snippet.safeNote}</span>
          </div>
          <Highlight theme={themes.vsDark} code={snippet.safe} language="rust">
            {({ className, style, tokens, getLineProps, getTokenProps }) => (
              <pre className={className} style={style}>
                {tokens.map((line, i) => (
                  <div {...getLineProps({ line })} key={i}>
                    {line.map((token, key) => (
                      <span {...getTokenProps({ token })} key={key} />
                    ))}
                  </div>
                ))}
              </pre>
            )}
          </Highlight>
        </div>
      </div>
      <div className="snippet-terminal terminal">
        <div className="terminal-head">
          <span className="terminal-dots">
            <span /> <span /> <span />
          </span>
          <span className="terminal-title">{snippet.compileErrorNote}</span>
        </div>
        <pre className="terminal-body">{snippet.compileError}</pre>
      </div>
    </div>
  );
}

function actionHeadline(
  form: ActionForm,
  meta: ExampleMeta,
  sender: Sender,
  accounts: AccountsResponse | null,
): string {
  const who = sender === "alice" ? "Alice" : "Bob";
  if (form.kind === "transfer") {
    const to = form.to === "alice" ? meta.walletLabels.alice : meta.walletLabels.bob;
    void accounts;
    return `${who} → transfer ${form.amount} to ${to}`;
  }
  if (form.kind === "deposit") {
    return `${who} → deposit ${form.amount}`;
  }
  return `${who} → withdraw ${form.amount}`;
}

function ExampleView({
  meta,
  accounts,
  backend,
}: {
  meta: ExampleMeta;
  accounts: AccountsResponse | null;
  backend: BackendStatus;
}) {
  const [addrs, setAddrs] = useState<Addrs>(null);
  const [setupBusy, setSetupBusy] = useState(false);
  const [setupErr, setSetupErr] = useState<string | null>(null);

  const [connected, setConnected] = useState<Sender>("bob");
  const [forms, setForms] = useState<ActionForm[]>(meta.initialForms);
  const [busy, setBusy] = useState<number | null>(null);

  const [vulnActivity, setVulnActivity] = useState<ActivityEntry[]>([]);
  const [safeActivity, setSafeActivity] = useState<ActivityEntry[]>([]);
  const [stateRefresh, setStateRefresh] = useState(0);

  useEffect(() => {
    setForms(meta.initialForms);
  }, [meta]);

  const runSetup = async () => {
    setSetupBusy(true);
    setSetupErr(null);
    setVulnActivity([]);
    setSafeActivity([]);
    try {
      const a = await setup(meta.id);
      setAddrs(a);
    } catch (e) {
      setSetupErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSetupBusy(false);
    }
  };

  const runAction = async (form: ActionForm, idx: number) => {
    if (!addrs || !accounts) return;
    setBusy(idx);
    const headline = actionHeadline(form, meta, connected, accounts);
    const action = form.kind;
    let args: string[];
    if (form.kind === "transfer") {
      args = [accounts[form.to], form.amount];
    } else {
      args = [form.amount];
    }

    try {
      const [vuln, safe] = await Promise.all([
        call(meta.id, "vulnerable", addrs.vulnerable, action, connected, args),
        call(meta.id, "safe", addrs.safe, action, connected, args),
      ]);
      const now = Date.now();
      setVulnActivity((xs) => [
        {
          id: now,
          succeeded: vuln.succeeded,
          headline,
          detail: vuln.detail,
          timestamp: now,
        },
        ...xs,
      ]);
      setSafeActivity((xs) => [
        {
          id: now + 1,
          succeeded: safe.succeeded,
          headline,
          detail: safe.detail,
          timestamp: now,
        },
        ...xs,
      ]);
      setStateRefresh((x) => x + 1);
    } catch (e) {
      const now = Date.now();
      const detail = e instanceof Error ? e.message : String(e);
      setVulnActivity((xs) => [
        { id: now, succeeded: false, headline, detail, timestamp: now },
        ...xs,
      ]);
      setSafeActivity((xs) => [
        { id: now + 1, succeeded: false, headline, detail, timestamp: now },
        ...xs,
      ]);
    } finally {
      setBusy(null);
    }
  };

  const updateForm = (idx: number, next: ActionForm) => {
    setForms((fs) => fs.map((f, i) => (i === idx ? next : f)));
  };

  const deployLabel = useMemo(() => {
    if (setupBusy) return "deploying…";
    if (addrs) return "redeploy + reset";
    return "deploy both contracts";
  }, [setupBusy, addrs]);

  return (
    <div className="app-view">
      <section className="app-intro">
        <h1>{meta.appName}</h1>
        <p className="tagline">{meta.tagline}</p>
        <p className="description">{meta.description}</p>
        <p className="feature-line">
          pvmsafe feature being demonstrated: <em>{meta.feature}</em>
        </p>

        {meta.liveChain && backend !== "offline" && (
          <div className="setup-row">
            <button
              className="setup-btn"
              onClick={runSetup}
              disabled={setupBusy || backend === "probing"}
            >
              {deployLabel}
            </button>
            {addrs && !setupBusy && (
              <span className="setup-hint">
                ✓ deployed · Alice has{" "}
                {meta.id === "erc20" ? "1000 tokens" : "1000 shares"}
              </span>
            )}
            {setupErr && <span className="err">{setupErr}</span>}
          </div>
        )}
        {meta.liveChain && backend === "offline" && (
          <p className="compile-only-note">
            static mirror · no backend reachable — the live deploy + attack
            flow needs a local <code>revive-dev-node</code> and the demo
            server. Clone the repo and run <code>demo/scripts/start-node.sh</code>
            then <code>demo/scripts/start-demo.sh</code> to interact. The code
            + compile-error panel below is the essential part.
          </p>
        )}
        {!meta.liveChain && (
          <p className="compile-only-note">
            compile-time scenario · no chain interaction — the point is that
            the vulnerable version doesn't make it past <code>cargo build</code>
          </p>
        )}
      </section>

      <Snippet snippet={meta.snippet} />

      {addrs && accounts && (
        <>
          <section className="wallet-bar">
            <div className="wallet-bar-label">connected wallet</div>
            <div className="wallet-bar-options">
              <WalletBadge
                name="Alice"
                short={formatAddr(accounts.alice)}
                active={connected === "alice"}
                onClick={() => setConnected("alice")}
              />
              <WalletBadge
                name="Bob"
                short={formatAddr(accounts.bob)}
                active={connected === "bob"}
                onClick={() => setConnected("bob")}
              />
            </div>
            <div className="wallet-bar-hint">
              transactions are signed by this account on both contracts at once
            </div>
          </section>

          <section className="actions">
            {forms.map((form, i) => {
              if (form.kind === "transfer") {
                return (
                  <TransferForm
                    key={i}
                    form={form}
                    update={(next) => updateForm(i, next)}
                    submit={() => runAction(form, i)}
                    busy={busy === i}
                    accounts={accounts}
                  />
                );
              }
              return (
                <AmountForm
                  key={i}
                  form={form}
                  update={(next) => updateForm(i, next)}
                  submit={() => runAction(form, i)}
                  busy={busy === i}
                  title={form.kind}
                  verb={form.kind}
                />
              );
            })}
          </section>
        </>
      )}

      {meta.liveChain && (
        <div className="instance-grid">
          <Instance
            meta={meta}
            kind="vulnerable"
            contract={addrs?.vulnerable ?? null}
            accounts={accounts}
            connected={connected}
            activity={vulnActivity}
            stateRefresh={stateRefresh}
          />
          <Instance
            meta={meta}
            kind="safe"
            contract={addrs?.safe ?? null}
            accounts={accounts}
            connected={connected}
            activity={safeActivity}
            stateRefresh={stateRefresh}
          />
        </div>
      )}
    </div>
  );
}

type BackendStatus = "probing" | "online" | "offline";

export function Demo() {
  const [active, setActive] = useState<Example>("erc20");
  const [accounts, setAccounts] = useState<AccountsResponse | null>(null);
  const [backend, setBackend] = useState<BackendStatus>("probing");
  const activeMeta = EXAMPLES.find((e) => e.id === active)!;

  useEffect(() => {
    getAccounts()
      .then((a) => {
        setAccounts(a);
        setBackend("online");
      })
      .catch(() => {
        setAccounts(null);
        setBackend("offline");
      });
  }, []);

  return (
    <div className="demo-page">
      <div className="demo-subnav">
        <span className="demo-subnav-label">example contract</span>
        <nav className="app-nav">
          {EXAMPLES.map((e) => (
            <button
              key={e.id}
              className={`app-tab ${active === e.id ? "active" : ""}`}
              onClick={() => setActive(e.id)}
            >
              {e.appName}
            </button>
          ))}
        </nav>
        <span className="demo-subnav-hint">
          <span className="brand-dot" />
          {backend === "offline"
            ? "static mirror · backend not reachable"
            : "pallet-revive dev-node · localhost:29999"}
        </span>
      </div>
      <ExampleView
        key={activeMeta.id}
        meta={activeMeta}
        accounts={accounts}
        backend={backend}
      />
    </div>
  );
}
