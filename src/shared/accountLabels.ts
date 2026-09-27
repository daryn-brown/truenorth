import type { AccountTypeId, ConnectorKind } from "../types/finance";

export const ACCOUNT_TYPE_LABELS: Record<AccountTypeId, string> = {
  chequing: "Chequing",
  savings: "Savings",
  brokerage: "Brokerage",
  tfsa: "TFSA",
  rrsp: "RRSP",
  fhsa: "FHSA",
  "401k": "401(k)",
  ira: "IRA",
  roth_ira: "Roth IRA",
  credit: "Credit",
  crypto: "Crypto",
  other: "Other",
};

export const CONNECTOR_LABELS: Record<ConnectorKind, string> = {
  manual: "Manual",
  snaptrade: "SnapTrade",
  simplefin: "SimpleFIN",
  questrade: "Questrade direct",
  teller: "Teller",
};
