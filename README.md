# vietqr-core

Core Rust offline tạo/đọc chuỗi **VietQR NAPAS 247** (EMVCo Merchant-Presented Mode,
profile chuyển khoản theo TCCS 03:2018/NHNN). Một workspace, một library,
**không dependency**, không mạng, filesystem, thread hay clock trong core.
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
- `parse(&str) -> Result<Parsed<'_>, Error>` mượn các field từ payload gốc;
  `validate(&str) -> Result<(), Error>` dùng cùng validation.
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
| 59 (tuỳ chọn) | 1–25 ASCII in được sau bỏ dấu; không tự thêm `NA` |
| 62.08 (tuỳ chọn) | 1–25 ký tự sau bỏ dấu: chữ/số ASCII, dấu cách và `-.,_@/():`; không emoji/control |
| TLV / tổng | mỗi value 1–99 byte; toàn payload tối đa **512 byte** (giới hạn profile local) |

**Bất định spec:** EMVCo MPM v1.1, bảng Additional Data Field Template,
định nghĩa Purpose of Transaction `62.08` tối đa **25**, Merchant Name `59` tối đa
**25**, Transaction Amount `54` tối đa **13**. Thực tế npm `@viet-qr/core@1.4.0`
đã kiểm tra ngày **2026-11-05** cắt memo/tên ở **50**, account tối đa 50 và
cho BIN 6–8 chữ số. Đây là khác biệt ecosystem, **không coi 50 là giới hạn chuẩn
62.08**. Crate chọn trần 25 thận trọng, lỗi thay vì cắt. Charset memo theo subset
npm đã kiểm tra. Trần account 50 là profile tương thích npm, card 19 và tổng 512
là giới hạn local; chưa xác nhận lại các trần này với tài liệu NAPAS chính thức.
Không tải trực tiếp tài liệu chuẩn/API do ràng buộc mạng của bước này; cần đối chiếu
bản TCCS/NAPAS ngân hàng đang áp dụng trước production. Không tuyên bố chứng nhận
EMVCo/NAPAS hoặc hỗ trợ toàn bộ merchant QR (không tự thêm MCC/merchant city).

Parser kiểm tra cấu trúc và ID trùng ở top-level, `38`, `38.01`, `62`; kiểm tra
ngữ nghĩa các field profile trên. Unknown tags được bỏ qua, value opaque
(không đệ quy hay kiểm tra ngữ nghĩa extension chưa biết), nhưng vẫn chịu TLV/CRC/
giới hạn tổng. Không nhận Unicode trực tiếp trong payload; generate xuất ASCII.
Không sửa CRC hỏng, không bỏ whitespace ngoài payload. Payload hợp lệ/CRC đúng
**không xác minh chủ tài khoản hay việc đã nhận tiền**. Không dùng tài khoản giả
trong ví dụ để chuyển tiền thật.

## Tests và cross-check

```sh
cargo test
cargo clippy -- -D warnings
cargo clippy --all-targets -- -D warnings
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
memo tiếng Việt. **Raw payload không byte-equal** vì npm luôn thêm `60=VN` và
thêm `59=NA` nếu thiếu tên. Script bỏ đúng các mặc định này, tính lại CRC bằng
npm; tests Rust assert byte-equal với cột core-profile, đồng thời parse raw npm
và so sánh các field. Parser npm cũng assert nhận cả hai profile.
`vietqr@1.1.9` (Casso/MIT) chỉ là client API; không gọi API, không dùng credentials.
Không có npm dependency hay `node_modules` trong repo.

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

Small, dependency-free Rust workspace for offline VietQR NAPAS 247 payloads:
TLV, CRC16-CCITT-FALSE, account/card generation, strict parsing and bank lookup.
See the example above. The conservative profile limits memo/name to 25 ASCII
characters and payloads to 512 bytes; npm compatibility differences and unverified
NAPAS-specific limits are documented above. Valid CRC is not payment confirmation.
Local-only, MIT, publishing disabled. Step 2 may add QR rendering/WASM bindings;
this step does not include images, browser integration, servers or payment calls.
