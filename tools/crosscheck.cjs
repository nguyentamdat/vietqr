// Run with NODE_PATH pointing to scratch node_modules; never contacts a payment API.
const assert = require('node:assert/strict');
const { generateVietQR, parseVietQR, calculateCRC16 } = require('@viet-qr/core');
const packagePath = require('node:path').join(require('node:path').dirname(require.resolve('@viet-qr/core')), '..', 'package.json');
assert.equal(JSON.parse(require('node:fs').readFileSync(packagePath, 'utf8')).version, '1.4.0');
const cases = [
  ['970436', '0000000000', false, undefined, undefined, undefined],
  ['970422', '0000000001', false, 50000, 'TEST USER', 'Order 001'],
  ['970415', '0000000000000000', true, 1, 'TEST CARD', 'Card test'],
  ['970407', 'TEST0001', false, undefined, 'TEST USER', 'Static memo'],
  ['970416', '0000000002', false, 9999999999999, undefined, '1234567890123456789012345'],
  ['970418', '0000000003', false, 2000, 'DANG THI A', 'Đóng tiền ăn'],
];
function fields(payload) {
  const result = [];
  for (let i = 0; i < payload.length;) {
    const length = Number(payload.slice(i + 2, i + 4));
    result.push([payload.slice(i, i + 2), payload.slice(i + 4, i + 4 + length)]);
    i += 4 + length;
  }
  return result;
}
console.log('# @viet-qr/core 1.4.0; 2026-11-05; fake accounts only');
console.log('# bin\taccount\tservice\tamount\tname\tinput memo\tnpm raw\tstrict profile');
for (const [bankId, accountNo, isCard, amount, accountName, content] of cases) {
  const raw = generateVietQR({bankId, accountNo, isCard, amount, accountName, content});
  assert.equal(parseVietQR(raw).isValid, true);
  // Strict (opt-in) profile difference: no default 59=NA, no 60=VN (merchant city).
  const body = fields(raw).filter(([id]) => id !== '63' && id !== '60' && !(id === '59' && accountName === undefined))
    .map(([id, value]) => id + String(value.length).padStart(2, '0') + value).join('') + '6304';
  const core = body + calculateCRC16(body);
  assert.equal(parseVietQR(core).isValid, true);
  assert.equal(raw === core, false); // Every reference payload contains 60=VN.
  console.log([bankId, accountNo, isCard ? 'card' : 'account', amount ?? '', accountName ?? '', content ?? '', raw, core].join('\t'));
}
