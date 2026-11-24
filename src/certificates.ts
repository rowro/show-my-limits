export function shellQuote(value: string): string {
  return "'" + value.replace(/'/g, "'\\''") + "'";
}

export function linuxTrustCommands(certificatePath: string): string {
  return `sudo install -D -m 644 -- ${shellQuote(certificatePath)} /usr/local/share/ca-certificates/show-my-limits.crt\nsudo update-ca-certificates`;
}

export function linuxReplacementTrustCommands(certificatePath: string, sha256: string): string {
  const digest = sha256.replace(/:/g, "").toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(digest)) throw new Error("Некорректный отпечаток нового CA");
  return `sudo install -D -m 644 -- ${shellQuote(certificatePath)} /usr/local/share/ca-certificates/show-my-limits-${digest}.crt\nsudo update-ca-certificates`;
}
