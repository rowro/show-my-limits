export function shellQuote(value: string): string {
  return "'" + value.replace(/'/g, "'\\''") + "'";
}

export function linuxTrustCommands(certificatePath: string): string {
  return `sudo install -D -m 644 -- ${shellQuote(certificatePath)} /usr/local/share/ca-certificates/show-my-limits.crt\nsudo update-ca-certificates`;
}
