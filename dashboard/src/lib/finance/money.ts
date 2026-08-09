export function exactMoney(mantissa: string, scale: number, currency: string) {
  const negative = mantissa.startsWith("-");
  const unsigned = BigInt(negative ? mantissa.slice(1) : mantissa);
  const divisor = scale > 2 ? 10n ** BigInt(scale - 2) : 1n;
  const scaled = scale > 2
    ? (unsigned + divisor / 2n) / divisor
    : unsigned * 10n ** BigInt(2 - scale);
  const whole = scaled / 100n;
  const fraction = String(scaled % 100n).padStart(2, "0");
  const grouped = String(whole).replace(/\B(?=(\d{3})+(?!\d))/g, ".");
  return `${negative && scaled !== 0n ? "-" : ""}${grouped},${fraction} ${currency}`;
}
