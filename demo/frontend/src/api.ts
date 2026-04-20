export type Example = "erc20" | "vault" | "effects";
export type Kind = "vulnerable" | "safe";
export type Sender = "alice" | "bob";

export type SetupResponse = {
  vulnerable: string;
  safe: string;
};

export type StateEntry = {
  label: string;
  value: string;
  role: "ledger" | "victim" | "attacker" | string;
};

export type StateResponse = {
  entries: StateEntry[];
};

export type CallResponse = {
  succeeded: boolean;
  detail: string;
};

export type AccountsResponse = {
  alice: string;
  bob: string;
};

export async function setup(example: Example): Promise<SetupResponse> {
  const r = await fetch("/api/setup", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ example }),
  });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export async function getState(
  example: Example,
  contract: string,
  kind: Kind,
): Promise<StateResponse> {
  const r = await fetch(
    `/api/state?example=${example}&contract=${contract}&kind=${kind}`,
  );
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export async function call(
  example: Example,
  kind: Kind,
  contract: string,
  action: string,
  sender: Sender,
  args: string[],
): Promise<CallResponse> {
  const r = await fetch("/api/call", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ example, kind, contract, action, sender, args }),
  });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export async function getAccounts(): Promise<AccountsResponse> {
  const r = await fetch("/api/accounts");
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

const MAX_U256 =
  "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

export function parseBalance(hex: string): bigint {
  const clean = hex.toLowerCase().startsWith("0x") ? hex.slice(2) : hex;
  return BigInt("0x" + (clean || "0"));
}

export function formatBalance(hex: string): string {
  const clean = hex.toLowerCase().startsWith("0x") ? hex.slice(2) : hex;
  const trimmed = clean.replace(/^0+/, "") || "0";
  if (trimmed === MAX_U256) return "2²⁵⁶ − 1";
  if (trimmed.length <= 18) {
    return BigInt("0x" + trimmed).toLocaleString("en-US");
  }
  const exp = trimmed.length * 4;
  return `≈ 2^${exp}`;
}

export function formatAddr(addr: string | null | undefined): string {
  if (!addr) return "…";
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}
