import { describe, expect, it } from 'bun:test';
import { exactMoney } from '../src/lib/finance/money';

describe('exactMoney', () => {
  it('keeps cents and groups large values', () => {
    expect(exactMoney('1750000', 2, 'EUR')).toBe('17.500,00 EUR');
  });

  it('rounds higher precision values to cents for display', () => {
    expect(exactMoney('162640064005', 7, 'EUR')).toBe('16.264,01 EUR');
    expect(exactMoney('-123455', 3, 'EUR')).toBe('-123,46 EUR');
  });

  it('pads values with a lower scale', () => {
    expect(exactMoney('42', 0, 'EUR')).toBe('42,00 EUR');
  });
});
