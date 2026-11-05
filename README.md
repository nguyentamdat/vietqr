# vietqr

Workspace Rust offline tạo/đọc chuỗi **VietQR NAPAS 247** (EMVCo Merchant-Presented
Mode, profile chuyển khoản theo TCCS 03:2018/NHNN), render SVG và WASM adapter.
Ba crate: `vietqr-core`, `vietqr-render`, `vietqr-wasm`. Core **không dependency**,
không mạng, filesystem, thread hay clock.
MIT; repo chỉ local, chưa quyết định public/private. `publish = false`.

## Dùng thử

Thêm path dependency ở project sử dụng:

```toml
[dependencies]
vietqr-core = { path = "../vietqr/crates/vietqr-core" }
```

```rust
use vietqr_core::{generate, parse, Service, Transfer};

fn main() -> Result<(), vietqr_core::Error> {
    let payload = generate(&Transfer {
        bank_bin: "970436",       // VCB; số tài khoản bên dưới là giả
        account: "0000000000",   // giữ số 0 đầu, không đổi sang integer
        service: Service::Account,
        amount: Some(50_000),
        name: Some("TEST USER"),
        memo: Some("Đóng tiền ăn"),
    })?;
    let parsed = parse(&payload)?;
    assert_eq!(parsed.memo, Some("Dong tien an"));
    assert_eq!(parsed.amount, Some(50_000));
    Ok(())
}
```

### API

- `generate(&Transfer) -> Result<String, Error>`; `Service::{Account, Card}`.
  Mặc định `Profile::Npm`: luôn thêm `60=VN`, `59=NA` khi không name;
  name/memo tối đa 50 sau bỏ dấu, **lỗi thay vì cắt**.
- `generate_with_profile(&Transfer, Profile::Strict)` giữ hành vi bước 1:
  không thêm tag 59/60 mặc định, name/memo tối đa 25, lỗi khi vượt.
  Signature `generate`, `Transfer`, `Parsed`, `Service` không đổi; caller muốn
  payload cũ cần chọn strict. `MAX_MEMO_LEN`/`MAX_NAME_LEN` vẫn là trần strict;
  `COMPAT_MAX_TEXT_LEN=50` là trần npm.
- `parse(&str) -> Result<Parsed<'_>, Error>` mượn các field từ payload gốc;
  `validate(&str) -> Result<(), Error>` dùng cùng validation, nhận cả hai profile (name/memo tối đa 50).
- `tlv::{encode, decode, Field}`: ID `u8` 0–99, độ dài byte 1–99,
  từ chối ID trùng trong mỗi template, header sai, thiếu byte và cắt giữa UTF-8.
- `crc16(&[u8]) -> u16`: CCITT-FALSE; generate tính trên cả `6304`,
  xuất 4 hex **viết hoa**; parse yêu cầu CRC là field cuối.
- `strip_vietnamese_diacritics`: chữ Việt dựng sẵn và dấu tổ hợp, `đ→d`, `Đ→D`;
  không tự xoá ký tự không hỗ trợ. Generate áp dụng cho memo và tên;
  không trim, không tự viết hoa, không cắt ngắn dữ liệu.
- `banks::{BANKS, by_bin, by_code}`: 18 ngân hàng, code không phân biệt hoa/thường.
  BIN chưa có trong bảng vẫn được dùng nếu đúng định dạng.
- `Error` là enum có kiểu riêng cho BIN/account/amount/memo/name, lỗi TLV/CRC,
  field thiếu/sai/trùng và vượt tổng độ dài. ID trong lỗi nested là ID cục bộ.

### Profile và giới hạn (cần đọc trước khi tích hợp ngân hàng)

| Trường | Quy tắc |
|---|---|
| 00, 01 | `00=01`; static `01=11` khi không amount, dynamic `01=12` khi có amount; parse yêu cầu nhất quán |
| 38 | `00=A000000727`, `01={00=BIN,01=account}`, `02=QRIBFTTA`/`QRIBFTTC` |
| BIN | đúng 6 chữ số ASCII; không khẳng định BIN đang hoạt động |
| account | 1–50 chữ/số ASCII, giữ nguyên, không bỏ dấu cách/gạch ngang |
| card | 1–19 chữ số ASCII (trần PAN); không kiểm tra Luhn, độ dài tối thiểu theo issuer hay tồn tại thẻ |
| 53, 54, 58 | `704`, amount nguyên VND `1..=9_999_999_999_999` (13 chữ số), `VN`; `Some(0)` là lỗi |
| 59 | 1–50 ASCII in được sau bỏ dấu ở npm, 1–25 ở strict; npm thêm `NA` nếu thiếu |
| 60 | npm luôn thêm `VN`; strict không thêm |
| 62.08 (tuỳ chọn) | 1–50 ký tự ở npm, 1–25 ở strict, sau bỏ dấu: chữ/số ASCII, dấu cách và `-.,_@/():`; không emoji/control |
| TLV / tổng | mỗi value 1–99 byte; toàn payload tối đa **512 byte** (giới hạn profile local) |

**Bất định spec:** EMVCo MPM v1.1, bảng Additional Data Field Template,
định nghĩa Purpose of Transaction `62.08` tối đa **25**, Merchant Name `59` tối đa
**25**, Transaction Amount `54` tối đa **13**. Thực tế npm `@viet-qr/core@1.4.0`
đã kiểm tra ngày **2026-11-05** cắt memo/tên ở **50**, account tối đa 50 và
cho BIN 6–8 chữ số. Đây là khác biệt ecosystem, **không coi 50 là giới hạn chuẩn
62.08**. Strict chọn trần 25 thận trọng; default npm chọn 50, cả hai lỗi thay vì cắt. Charset memo theo subset
npm đã kiểm tra. Trần account 50 là profile tương thích npm, card 19 và tổng 512
là giới hạn local; chưa xác nhận lại các trần này với tài liệu NAPAS chính thức.
Không tải trực tiếp tài liệu chuẩn/API do ràng buộc mạng của bước này; cần đối chiếu
bản TCCS/NAPAS ngân hàng đang áp dụng trước production. Không tuyên bố chứng nhận
EMVCo/NAPAS hoặc hỗ trợ toàn bộ merchant QR (không tự thêm MCC; merchant city default chỉ để tương thích npm).

Parser kiểm tra cấu trúc và ID trùng ở top-level, `38`, `38.01`, `62`; kiểm tra
ngữ nghĩa các field profile trên. Unknown tags được bỏ qua, value opaque
(không đệ quy hay kiểm tra ngữ nghĩa extension chưa biết), nhưng vẫn chịu TLV/CRC/
giới hạn tổng. Không nhận Unicode trực tiếp trong payload; generate xuất ASCII.
Không sửa CRC hỏng, không bỏ whitespace ngoài payload. Payload hợp lệ/CRC đúng
**không xác minh chủ tài khoản hay việc đã nhận tiền**. Không dùng tài khoản giả
trong ví dụ để chuyển tiền thật.

## Tests và cross-check

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --check
```

CRC check vector chuẩn: `123456789 → 29B1`. Có fixture npm, boundary/error tests,
và 432 tổ hợp round-trip (18 ngân hàng × 2 service × 4 amount × 3 độ dài account).

Cross-check tái lập, dependency/cache chỉ ở `/tmp`:

```sh
mkdir -p /tmp/task336-crosscheck
cd /tmp/task336-crosscheck
npm install --cache /tmp/task336-npm-cache --no-audit --no-fund vietqr@1.1.9 @viet-qr/core@1.4.0
cd /path/to/vietqr
NODE_PATH=/tmp/task336-crosscheck/node_modules node tools/crosscheck.cjs > /tmp/task336-crosscheck/vectors.tsv
diff -u crates/vietqr-core/tests/fixtures/npm-vectors.tsv /tmp/task336-crosscheck/vectors.tsv
cargo test npm_reference_vectors
```

6 input giống nhau: static, dynamic, card, account chữ/số, amount/memo sát trần,
memo tiếng Việt. **Default byte-equal RAW npm cả 6 vector**. Strict byte-equal
cột strict-profile (bỏ đúng default `60=VN` và `59=NA` khi thiếu name, tính lại CRC).
Không cần truncate để đạt byte-equality cho 6 vector; input vượt trần vẫn lỗi,
khác npm cắt âm thầm. Cả parser Rust và npm nhận hai profile.
`vietqr@1.1.9` (Casso/MIT) chỉ là client API; không gọi API, không dùng credentials.
Không có npm dependency hay `node_modules` trong repo.

## SVG và WASM

`vietqr-render::render_svg(&str) -> Result<String, QrError>` dùng
[`qrcode` 0.14](https://crates.io/crates/qrcode) (không default features; chỉ svg).
QR thường, ECC **M**, quiet zone **4 module**, đen/trắng, viewBox một unit/module;
scale bằng CSS khi hiển thị. Chuỗi bắt đầu `<svg`, không có XML header. Renderer
nhận chuỗi bất kỳ, **không validate VietQR**; gọi `parse` trước nếu cần.
Không nhúng payload dưới dạng markup; không có PNG, logo hay màu tuỳ biến.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --locked # nếu chưa có
wasm-pack build crates/vietqr-wasm --target web --out-dir ../../target/wasm-web --locked
# Test Node dùng build riêng, không dùng web glue trong require():
wasm-pack build crates/vietqr-wasm --target nodejs --out-dir /tmp/vietqr-node --locked
```

Release dùng `opt-level="z"`, LTO, một codegen unit và strip. Build artifacts không
commit. `vietqr-wasm` dùng wasm-bindgen; API JS **chuỗi vào/ra**, lỗi throw string:

- `generate(jsonOptions) -> payload`: `bank_bin`, `account` là chuỗi bắt buộc,
  `service: 'account' | 'card'` bắt buộc; optional `amount` (integer VND), `name`,
  `memo`, `profile: 'npm' | 'strict'` (default npm). Reject unknown fields/types.
  Optional amount/name/memo được phép thiếu hoặc null. Không dùng BigInt cho JSON;
  MAX_AMOUNT < Number.MAX_SAFE_INTEGER. Không tự chọn bank/service hay trim.
- `parse(payload) -> json`: `bank_bin`, `account`, `service`, `amount`, `name`,
  `memo`; optional vắng trả null. Không trả profile/city; không lưu text gốc có dấu.
- `render_svg(payload) -> svg`: giống Rust renderer; lỗi khi vượt sức chứa QR.
- `banks() -> json`: mảng `{bin, code, name}` từ snapshot 18 ngân hàng trong core.

```js
import init, { generate, parse, render_svg, banks } from './vietqr_wasm.js';
await init(); // Web target: tải .wasm bên cạnh JS qua import.meta.url
const payload = generate(JSON.stringify({
  bank_bin: '970436', account: '0000000000', service: 'account',
  amount: 50000, memo: 'Đóng tiền ăn', // profile: 'strict' để dùng profile cũ
}));
const fields = JSON.parse(parse(payload));
const svg = render_svg(payload);
const bankList = JSON.parse(banks());
```

### Bước 3: Next.js static export / GitHub Pages

Repo demo riêng `nguyentamdat/my-site`: `output: 'export'`, `basePath: '/my-site'`.
Copy `vietqr_wasm.js` và `vietqr_wasm_bg.wasm` từ build **web** vào
`public/wasm/` (giữ cùng thư mục); `.d.ts` dùng khi cần type checking.
Trong client component (`'use client'`), load module và `await init(...)` trong
`useEffect`/event trước khi gọi API, **không ở SSR/build-time**. Nếu load JS trực
tiếp từ public, dùng browser dynamic import URL `/my-site/wasm/vietqr_wasm.js`
(ví dụ `import(/* webpackIgnore: true */ url)` với Webpack); không để bundler
resolve file public như source dependency. Init rõ URL:
`await module.default({ module_or_path: '/my-site/wasm/vietqr_wasm_bg.wasm' })`.
Next basePath không tự prefix chuỗi URL public; không fetch `/wasm/...` ở root.
Có thể để init mặc định resolve WASM cạnh JS, nhưng URL JS vẫn phải có basePath.
Không cần server/API route cho demo; chưa sửa/chạy repo Next.js ở bước này.

## Nguồn

- [EMVCo QR Code Specifications — Merchant-Presented Mode v1.1](https://www.emvco.com/emv-technologies/qr-codes/): TLV, CRC, tag 54/59/62.08.
- **TCCS 03:2018/NHNN**, Đặc tả kỹ thuật QR Code hiển thị từ phía đơn vị chấp nhận thanh toán
  (Quyết định 1928/QĐ-NHNN, 05/10/2018); [NHNN](https://www.sbv.gov.vn/).
- [NAPAS VietQR](https://napas.com.vn/): bối cảnh chuyển nhanh NAPAS 247; không phải xác nhận tuân thủ.
- [@viet-qr/core 1.4.0](https://www.npmjs.com/package/@viet-qr/core/v/1.4.0), MIT/Zeforc:
  source runtime đã tải từ npm registry ngày 2026-11-05; bảng BIN/code/name lấy trực tiếp
  từ `banks` export (18 entry đầu), không tự suy đoán BIN. Có thể cũ/đổi thương hiệu.
- [vietqr 1.1.9](https://www.npmjs.com/package/vietqr/v/1.1.9), MIT/Casso.
- [VietQR banks API](https://api.vietqr.io/v2/banks): nguồn cập nhật tham khảo,
  **không gọi** trong bước này.

## English

Offline VietQR Rust workspace: dependency-free core, QR SVG renderer (ECC M,
quiet zone 4), and wasm-bindgen JSON-string bindings. Default generation matches
npm defaults (59=NA if unnamed, 60=VN, memo/name limit 50); opt into
`generate_with_profile(..., Profile::Strict)` for the original 25-character profile
without defaults. Neither profile truncates. Parsing accepts both profiles.
Payload limit: 512 bytes. Unverified NAPAS-specific limits remain documented above.
Valid CRC is not payment confirmation. Local-only, MIT, publishing disabled.
No PNG, server, payment calls or demo UI in this repository.
