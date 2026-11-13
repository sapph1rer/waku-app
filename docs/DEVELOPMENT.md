# Waku — บันทึกการพัฒนา

> เอกสารสำหรับคนพัฒนาต่อ (รวมถึง Claude Code): วิธีทำงานภายใน, แหล่งข้อมูลแต่ละเว็บ, บันทึกแต่ละรอบแก้ และสิ่งที่ยังไม่ได้ทดสอบ
> คู่มือผู้ใช้อยู่ที่ [README.md](../README.md)

แอปดูอนิเมะซับไทยบนเดสก์ท็อป เขียนด้วย Rust + egui — ต่อยอดจาก [Waku-cli](https://github.com/sapph1rer/Waku-cli)
ดึงจาก **หลายแหล่งพร้อมกัน** ถ้าแหล่งหนึ่งช้า/ล่ม/เซิร์ฟเวอร์เสีย จะสลับไปหาตอนเดียวกันจากแหล่งอื่นให้อัตโนมัติ
หน้าตา: sidebar ไอคอนด้านซ้าย, hero ภาพพื้นหลังเฟดเข้าดำ, แท็บ Episodes / Details / Other Sources / More Like This,
คอลัมน์ซีซั่น + การ์ดตอนแบบมี thumbnail และ progress bar — UI ภาษา **English (ค่าเริ่มต้น) / ไทย** เปลี่ยนได้ใน Settings

## ฟีเจอร์

- **Home** — hero แนะนำ (หมุนอัตโนมัติ), แถว Continue Watching, แถว "New on <แหล่ง>" ของทุกแหล่ง
- **Browse** — grid ของแต่ละแหล่ง สลับด้วย chip ที่มีจุดสถานะ เขียว/เหลือง/แดง + โหลดเพิ่ม
- **ข้อมูลเรื่องจาก AniList / Kitsu / Jikan** (พอร์ตจาก AnimeShelf): ปก, banner, เรื่องย่อ, แนวเรื่อง, สตูดิโอ, คะแนน, ชื่อตอน, ภาพตอน, วันฉาย, ความยาว และนับถอยหลังตอนถัดไป — จับคู่อัตโนมัติจากชื่อบนเว็บ (ตัดภาษาไทย/คำต่อท้ายออก + ใช้ slug ใน URL) แคชไว้ใน `meta.json`
- **Identify แบบ Jellyfin** — คลิกขวาที่การ์ดหรือ hero → Identify… / Refresh metadata / Clear identification; หน้าต่าง Identify ค้น AniList ได้เอง หรือวาง AniList ID / ลิงก์ — match ที่เลือกเองจะไม่ถูกทับ และใช้กับเรื่องเดียวกันบนแหล่งอื่นด้วย
- **On Air** — ตารางฉายรายสัปดาห์จาก AniList ตามเวลาเครื่อง (จ.–อา.), สถานะกำลังฉาย / อีกกี่ชม. / ฉายแล้ว, เส้น "Now", ตัวกรอง + "เรื่องของฉัน"; กด Find เพื่อเปิดเรื่องที่จับคู่ไว้แล้ว หรือค้นจากทุกแหล่ง
- **ค้นหาทุกแหล่งพร้อมกัน** — ผลขึ้นทีละแหล่งตามที่ตอบกลับ, fuzzy search จาก Waku-cli (`re zero` / `rezero` / ชื่อไทย) + index จาก sitemap
- **หน้าเรื่อง** — poster, เรื่องย่อ, แท็ก, รายการตอน (แยกซีซั่น, เรียงกลับได้), ปุ่ม "ดูต่อ: ตอนที่ N", รายการของฉัน
- **แหล่งอื่น** — หาเรื่องเดียวกันจากแหล่งอื่น พร้อม % ความเหมือนของชื่อ (เช็กเลขซีซั่นด้วย กันจับผิดภาค)
- **player ในตัว** (rmp + libmpv, หน้าตาแบบ AnimeShelf) หรือเลือกเปิดด้วย mpv ภายนอกได้ในหน้าตั้งค่า
- **ซับ / พากย์ แยกแทร็ก** — สวิตช์บนรายการตอน, จำว่าดูแทร็กไหนล่าสุด, failover หาแทร็กเดียวกันจากแหล่งอื่น
- **ดูถึงไหนแล้ว** — progress ต่อตอนจากตำแหน่งที่ดูจริง, ✓ ตอนที่ดูจบ, "ดูค้างไว้" สำหรับตอนที่ยังไม่จบ
- **Failover อัตโนมัติ**: ทุกเซิร์ฟเวอร์ของแหล่งนี้ → (เปิด/ปิดได้) หาตอนเดียวกันในแหล่งอื่นตามลำดับที่ตั้งไว้ → เล่นต่อ
- **เลือกเซิร์ฟเวอร์เอง** — ปุ่มขวาของแต่ละตอน บอกชนิดสตรีม (ตรง / yt-dlp / ทางสุดท้าย) และความละเอียดที่มี
- **ตั้งค่า** — ภาษา (English/ไทย), path ของ player, คุณภาพ, ขนาด UI, ลำดับแหล่ง, ตรวจสถานะ, **ปุ่มทดสอบแหล่ง** (บอกว่าพังขั้นไหน)

## Build (Windows)

```powershell
cargo build --release
```

ไฟล์อยู่ที่ `target\release\` วางโครงแบบนี้:

```
waku-app.exe
waku-browser.exe     ← ตัวช่วยผ่าน Cloudflare + อ่านหน้า player (build มาพร้อมกัน; ต้องมี WebView2 Runtime)
libmpv-2.dll         ← player ในตัว (ดู ../rmp/third_party/libmpv/README.md หรือ scripts/get-libmpv.ps1)
mpv\yt-dlp.exe       ← แนะนำ (ok.ru, dailymotion ฯลฯ)
mpv\mpv.exe          ← ไม่บังคับ: ใช้เมื่อเลือก "Play in: external player"
fonts\*.ttf          ← ไม่บังคับ: วางฟอนต์ไทยที่ชอบ (เช่น Kanit / Noto Sans Thai, ไฟล์ที่มีคำว่า Bold = ตัวหนา)
```

ถ้าไม่วางฟอนต์ แอปใช้ Leelawadee UI ของ Windows (ญี่ปุ่นใช้ Yu Gothic / Meiryo)

### โหมด console (ไว้ดีบักแหล่ง)

```powershell
waku-app.exe --probe all        # ทดสอบทุกแหล่ง: ล่าสุด → หน้าเรื่อง → player → สตรีม
waku-app.exe --probe anifume    # ทดสอบแหล่งเดียว พร้อม trace ทุก hop
waku-app.exe --search frieren   # ค้นหาทุกแหล่งแล้วพิมพ์ผล
```

## แหล่งข้อมูล — `sources.toml`

ครั้งแรกที่เปิดแอปจะสร้าง `%APPDATA%\waku\sources.toml` (ปุ่ม "เปิด sources.toml" ในหน้าตั้งค่า) แก้แล้วกด "โหลดแหล่งใหม่" ไม่ต้อง compile ใหม่

| id | เว็บ | หน้ารายการ / ค้นหา / ตอน | วิดีโอ (ทดสอบ 11 พ.ย. 2026 จากเน็ตไทย) |
|---|---|---|---|
| `waku` | anime-waku.com | ผ่าน **Cloudflare** ด้วย `waku-browser` · แยก ซับ/พากย์ ถูก | DooPlay API → nya.animenani.com → HLS (`/o/<id>/a` แบบ base64) ✅ เล่นผ่าน HLS proxy |
| `sugoi` | anime-sugoi.net | หน้าเดียวมีทุกตอน (`data-ep` + `data-src`) → ตอนเป็น `…/#wk-ep=N` | playhdhls (vdohide) → `vdohls.com/<id>/playlist.m3u8`, segment ปลอมเป็น `.jpeg` ✅ |
| `anifume` | anifume.com | ✅ | `$.ajax` (ต้องส่ง `X-Requested-With`) → `/player/…` → JW `.mp4` 720p/360p ✅ |
| `animeruka` | animeruka.com | ✅ แยก ซับ/พากย์ (`-thai-ep-`) | animemami → `maimeorder.com/hls/….txt` (HLS ปลอมนามสกุล) ✅ · สำรอง ok.ru (yt-dlp) · abyss |
| `fairy` | fairyanime.net | ✅ หน้า "… เสียงไทย" = พากย์ทั้งหน้า | สคริปต์ `/base/<id>/` สร้างลิงก์ player 6 ตัว → JS obfuscate หลายชั้น → ใช้ `waku-browser` เล่นแล้วดักลิงก์ ✅ (`anime.tonytonychopper.net/file2/<id>/`) |
| `hareshi` | hareshi.net | **ปิดถาวร** — กลายเป็นเว็บรวมช่องทางดูลิขสิทธิ์ ไม่มีวิดีโอให้เล่น | – |

ถ้าปุ่มทดสอบฟ้องขั้นไหน ให้ใส่ค่านั้นใน profile ของเว็บนั้น เช่น:

```toml
[[source]]
id = "anifume"
name = "Anifume"
base = "https://anifume.com"        # เว็บย้ายโดเมน → แก้บรรทัดนี้บรรทัดเดียว
latest_paths = ["/", "/anime/"]
series_path  = '^/anime/[^/]+/?$'   # regex ของ path หน้าเรื่อง
episode_path = '^/ep/[^/]+/?$'      # regex ของ path หน้าตอน
card_selector = "article.item"      # กล่องการ์ดในหน้า listing
poster_selector = ".poster img"
synopsis_selector = ".wp-content p"
sitemap_index = true                # ค้นหาแม่นขึ้นด้วย index จาก sitemap
```

## โครงสร้าง

```
crates/waku-core   ไลบรารี (ไม่มี GUI) — ใช้ทำ CLI ตัวใหม่ได้
  source.rs        trait Source { latest, search, detail, servers, resolve }
  wp.rs            WpSource: engine ทั่วไปสำหรับเว็บ WordPress (DooPlay/heuristic) อ่าน config จาก profile
  stream.rs        sniff สตรีม, atob/base64, หาความละเอียดพี่น้อง (พอร์ตจาก Waku-cli)
  media.rs         ตัวหาวิดีโอแบบไล่ลึก 5 ชั้น: JW sources, $.ajax, Inertia, base64 กลับด้าน, vdohide, probe เนื้อไฟล์
  browser.rs       client ของ waku-browser (Cloudflare + เล่นหน้า player จริงแล้วดักลิงก์วิดีโอ)
  health.rs        จำว่า host ไหนเล่นได้/พัง (hosts.json) → เรียงเซิร์ฟเวอร์ที่น่าจะเล่นได้ขึ้นก่อน
  bin/waku-agent   ตัวช่วย dev: รับคำสั่งเป็นไฟล์ JSON แล้วดึงเว็บ/ทดสอบแหล่งจากเน็ตของเครื่องนั้น
  manager.rs       SourceManager: health, search_all, alternatives, match_episode, streams, diagnose
  player.rs        เรียก mpv/vlc (+ yt-dlp hook), ตรวจ exit code 2 เพื่อ failover
  store.rs         settings.json / library.json (รายการของฉัน, ประวัติ)
  meta/            AniList (search / by id / schedule), Kitsu, Jikan, การจับคู่ชื่อ, MetaStore (meta.json)
crates/waku-browser  waku-browser.exe — หน้าต่าง WebView2 ซ่อนอยู่ คุยผ่าน stdin/stdout (JSON ทีละบรรทัด)
crates/waku-app    GUI egui 0.36
  app.rs           state, background jobs (thread + channel), ลำดับการเล่น + failover
  i18n.rs          tr("English", "ไทย") / trf!(…) — เพิ่มภาษาใหม่ = เพิ่ม variant + argument
  pages.rs         Home / Browse / Search / My List / History / Settings
  detail.rs        หน้าเรื่อง: hero, แท็บ, คอลัมน์ซีซั่น, การ์ดตอน, ตัวเลือกเซิร์ฟเวอร์
  metaui.rs        ใช้ metadata กับการ์ด/หน้าเรื่อง, เมนูคลิกขวา, หน้าต่าง Identify, คิวดึงข้อมูลเบื้องหลัง
  onair.rs         แท็บ On Air
  widgets.rs       backdrop + gradient, การ์ด poster/ตอน, แท็บ, sidebar, ปุ่ม, shimmer, segmented (ซับ/พากย์), problem_card
  playerview.rs    player ในตัว (rmp-egui + libmpv) ต่อกับลำดับการลองลิงก์ / failover
  theme.rs         สี, ฟอนต์ไทย/ญี่ปุ่น, icon
```

ถ้าเว็บไหนไม่ใช่ WordPress (เช่นเป็น SPA ที่โหลดข้อมูลผ่าน JSON API) ให้เขียน struct ใหม่ที่ `impl Source`
แล้วเพิ่มใน `SourceManager::from_profiles` (มีคอมเมนต์จุดที่ต้องใส่ไว้แล้ว) — UI และ failover ใช้ได้ทันที

## Metadata — หมายเหตุ

- AniList จำกัด ~90 request/นาที: การ์ดในหน้าแรกถูกจับคู่ทีละเรื่องในคิวเบื้องหลัง ส่วนหน้าเรื่องที่เปิดอยู่ใช้ thread แยกจึงไม่ต้องรอคิว
- เรื่องที่ชื่อบนเว็บเป็นภาษาไทยล้วนและ URL ไม่มี slug โรมาจิ จะจับคู่อัตโนมัติไม่ได้ → ใช้ Identify
- ชื่อตอนใช้กับ **ซีซั่นแรกของหน้านั้น** เท่านั้น (AniList แยก entry ต่อซีซั่น) — หน้าที่รวมหลายซีซั่นให้ Identify เป็นภาคที่ต้องการ
- ทดสอบกับเซิร์ฟเวอร์จำลองได้ด้วย `WAKU_ANILIST`, `WAKU_KITSU`, `WAKU_JIKAN`
- ไม่ได้พอร์ต AniDB เพราะต้องลงทะเบียน client name ก่อน

## การเล่นวิดีโอ — ทำงานอย่างไร

1. **หาเซิร์ฟเวอร์** ของตอน (`servers`) แล้ว resolve ทุกตัวพร้อมกัน (`media::find_media` ไล่ iframe / ajax / JW / base64 / probe เนื้อไฟล์)
2. **เรียงลำดับ**: ลิงก์ตรง → yt-dlp → ไม่รู้จัก, ในกลุ่มเดียวกันเรียงตามคะแนน host จาก `hosts.json` (เล่นได้บ่อย = ขึ้นก่อน, เพิ่งพังใน 10 นาที = ถอยหลัง)
3. เซิร์ฟเวอร์ที่ยังเป็น "หน้า player" (obfuscate) → ถ้ามี `waku-browser.exe` จะให้เบราว์เซอร์ที่ซ่อนอยู่เปิดหน้านั้น กด play (ปิดเสียง) แล้วดักคำขอ `.m3u8/.mp4/…` (กรองโฆษณาออก) ใช้เวลา 3–25 วินาที ทำเฉพาะเซิร์ฟเวอร์ที่กำลังจะเล่น
4. ส่งให้ player ในตัว (libmpv): HLS ทุกตัววิ่งผ่าน **HLS proxy ของ rmp** (ถอด playlist ที่ปลอมเป็น .txt/base64/JSON, ตัด header รูปปลอมหน้า segment, ใส่ Referer/Origin ให้ทุก request)
5. เล่นไม่ได้ → ลอง Referer ถัดไป → เซิร์ฟเวอร์ถัดไป → (ถ้าเปิด auto-failover) หาตอนเดียวกัน **แทร็กเดียวกัน (ซับ/พากย์)** จากแหล่งอื่น
6. ระหว่างเล่น สตรีมหลุดจะ resolve ใหม่อัตโนมัติ ≤ 3 ครั้ง, บันทึกตำแหน่งทุก 10 วินาที (ดูต่อจากเดิม + progress bar บนการ์ดตอน)

### waku-browser (Cloudflare + player ที่ซ่อนลิงก์)

- ต้องวาง `waku-browser.exe` ไว้ข้าง `waku-app.exe` (cargo build ใส่ให้ใน `target\release` อยู่แล้ว) และเครื่องต้องมี **WebView2 Runtime** (Windows 10/11 มีมาแล้ว)
- เปิดครั้งแรกเมื่อจำเป็น (เจอ 403 Cloudflare หรือหน้า player ที่อ่านไม่ออก) แล้วค้างไว้ใช้ต่อ, cookie เก็บที่ `%LOCALAPPDATA%\Waku\browser`
- ถ้า Cloudflare ต้องการให้คลิก (Turnstile) เกิน 8 วินาที หน้าต่าง "Waku — browser check" จะโผล่ขึ้นมาให้คลิก แล้วซ่อนเองเมื่อผ่าน
- host ที่เคยเจอ Cloudflare จะใช้เบราว์เซอร์ตรงๆ ทันที (ภายในเว็บเดียวกันใช้ `fetch` ในหน้า → เร็ว)

## การทดสอบ

| อะไร | วิธี | ผลล่าสุด |
|---|---|---|
| unit test (parser, media, health, ตัดชื่อการ์ด) | `cargo test -p waku-core` | 29 ผ่าน |
| แต่ละแหล่งจากเน็ตจริง | `waku-agent <dir>` แล้ววางไฟล์ `req/x.json` เช่น `{"cmd":"streams","source":"fairy","url":"…","deep":true}` → ผลที่ `resp/x.txt` (คำสั่ง: `latest`, `search`, `detail`, `streams`, `diagnose`, `bfetch`, `sniff`) | ทุกแหล่งยกเว้น hareshi ได้ลิงก์ |
| ลิงก์เล่นได้จริงไหม | `rmp\target\release\examples\mpv_probe.exe <url> 12` + env `RMP_PROBE_REFERER`, `RMP_PROBE_HEADERS`, `RMP_PROBE_HLS=hls` | waku (nya), sugoi (vdohls), animeruka (maimeorder) เล่น 1080p ได้ |
| ทั้งแอป | `cargo run --release -p waku-app` แล้วกดดูตอน | ดูหัวข้อ "ยังไม่ได้ทดสอบ" |

### รอบแก้ 11 พ.ย. 2026 (หลังลองบนคอมที่ Windows เป็นโหมดสว่าง และไม่มี libmpv)

- **ธีม**: egui เก็บ style แยกตามธีมของ Windows และตามระบบเป็นค่าเริ่มต้น → บนเครื่องโหมดสว่าง ปุ่ม/ช่องตัวเลข/เมนูเป็นสีขาว
  ตอนนี้ล็อกเป็นธีมมืดทั้งสองแบบ (`theme.rs`, และ `rmp_egui::Theme::visuals()` สำหรับแผงของ player เอง)
- **ประวัติ / Continue Watching**: บันทึกเมื่อภาพเดินจริง ≥ 4 วินาทีเท่านั้น และบันทึกเป็น **แหล่งที่เล่นได้จริง** (หลัง failover)
  หนึ่งเรื่องมีรายการเดียวไม่ว่าดูจากเว็บไหน (`text::same_show`) และล้างรายการซ้ำเก่าตอนเปิดแอป
- **ลิงก์ที่ "เปิดได้" แต่ภาพไม่เดิน** ภายใน 15 วินาที นับเป็นพัง → ลองลิงก์/เซิร์ฟเวอร์/แหล่งถัดไปเอง
- **ไม่มี libmpv**: เอนจิน native ก็ใช้ HLS proxy แล้ว; หน้า Settings มีปุ่ม "Download libmpv" (โหลด ~30 MB จาก GitHub ของ
  shinchiro แล้วแตกด้วย `tar.exe` ของ Windows วางข้างโปรแกรม ใช้ได้ทันทีไม่ต้องเปิดใหม่)
- **ลำดับ failover** เรียงตามแหล่งที่เล่นได้ดีช่วงหลัง (`health::source_score`) ก่อนลำดับใน Settings
- **ค้นหา**: ช่องค้นหาใหม่, ค้นระหว่างพิมพ์ (หยุดพิมพ์ 0.55 วิ, ≥ 3 ตัวอักษร), ค้นหาล่าสุด, ทางลัด "หาแหล่งอื่นของเรื่องที่ดูอยู่"
- **Home**: carousel มี state ของตัวเอง (หยุดนับเมื่อเมาส์ชี้, กดจุด/ลูกศรได้, cross-fade แทนการกะพริบดำ)
- `WAKU_PAGE=search|settings|browse|history|mylist|onair` เปิดหน้านั้นทันที (ไว้ทดสอบ)

### รอบแก้ 11 พ.ย. 2026 (ช่วงเย็น) — แหล่งวิดีโอ + GUI

ดึงจากตัวเล่นหลักของแต่ละเว็บ (ทดสอบบนเครื่องจริงผ่าน `waku-agent` คำสั่ง `survey`):

| เว็บ | ตัวเล่นหลัก | ความชัด |
|---|---|---|
| Anime-Waku | `หลัก` → `/player/…` → nya.animenani (bootstrap base64 กลับด้าน) → `cat.animenani.com/o/<id>/a` HLS, segment ปลอมเป็น `.webp` | 1080p |
| Anime-Sugoi | playhdhls → `vdohls.com/<id>/playlist.m3u8` (มีเสียงไทย/ญี่ปุ่น/อังกฤษแยก) | 1080p |
| AnimeRuka | `หลัก` → `cdn2.maimeorder.com/hls/….txt` | ตามเว็บ |
| Anifume | `/player/…` JW sources (720p, 360p) — เลือกตัวชัดสุดเสมอ | 720p (สูงสุดที่เว็บมี) |
| FairyAnime | `playback/v/<vid>` → `anime.tonytonychopper.net/v2/<id>` → `file2/<id>/` HLS ต้องส่ง Referer = หน้า v2 + `Origin` + `Sec-Fetch-*` แบบ XHR (`media.rs::tonytonychopper`) | 1080p |

แก้บั๊กสำคัญ: `decode_playlist_head` slice สตริงกลางตัวอักษรตอนเจอไฟล์ binary (รูป poster) → panic ในเธรด → ตอนใหม่ล่าสุดของ
Anime-Waku "หาเซิร์ฟเวอร์ไม่เจอ" ทั้งที่ตัวเล่นหลักใช้ได้ (ตอนนี้ panic ของ resolver จะแสดงเป็น error แทน)

ความเร็ว: Cloudflare — หลังจาก waku-browser ผ่าน check แล้วจะเอา cookie `cf_clearance` + User-Agent ของมันมาใช้กับ HTTP ธรรมดา
(หน้าละ ~0.1 วิ แทน 3 วิ+); รายการหน้าแรกเก็บ cache ไว้ (`%LOCALAPPDATA%\waku\latest\*.json`) เปิดแอปมาเห็นทันที; การหาลิงก์ตอบกลับ
ทันทีที่ได้ลิงก์ตรง (รอเซิร์ฟเวอร์อื่นอีกแค่ 2.5 วิ, สูงสุด 30 วิ)

GUI: โลโก้แอป (`crates/waku-app/assets`, ฝังใน exe ด้วย `build.rs`/winresource), หน้า Loading ตอนเปิด, ปุ่ม/ไอคอนเล่นวาดเอง
(อยู่กลางจริง), hero ต่ำลง + โหลดรูปสไลด์ข้างๆ ล่วงหน้า (แก้รูป/ปุ่มไม่ขึ้นหลังกดลูกศร), ปุ่ม ✕ ลบออกจาก "ดูต่อ",
ตั้งค่าการเล่นเหลือแค่ที่จำเป็น (ที่เหลืออยู่ใน "ขั้นสูง"), เมนูคลิกขวาในวิดีโอแบบใหม่, Info บอกเว็บ/เซิร์ฟเวอร์/โฮสต์ที่กำลังเล่น

ทดสอบด้วยรีโมต: ตั้ง `WAKU_DEV_DIR=<โฟลเดอร์>` แล้วเขียนคำสั่งลง `<โฟลเดอร์>\cmd.txt` (`shot`, `click X Y`, `open SRC URL`,
`play N`, `state` …) — ดู `src/dev.rs`

### รอบแก้ 11 พ.ย. 2026 (กลางคืน) — Anime-Waku ตอนไม่ขึ้น / เปิดช้า (ทดสอบบนคอม Windows จริงแล้ว)

- **ตอนเป็น 0 + ชื่อรองเป็น "อนิเมะอัพเดทล่าสุด"**: waku-browser คืน HTML ของหน้าก่อนหน้า (job ใหม่เริ่มก่อนหน้าเดิมเปลี่ยน) →
  ตอนนี้ job จำ `start_href` และไม่รับ HTML จากหน้าเก่า; DooPlay ลอง wp-json **v1** ก่อน (v2 ช้า/ไม่ตอบ)
- **เปิดวิดีโอช้า 15–20 วิ**: คอมทดสอบมี IPv6 ที่ใช้ไม่ได้ (connect ค้างจน timeout) และ ureq ใน rmp ลอง IPv6 ก่อนทีละที่อยู่ →
  `rmp_core::dns::V4First` (IPv4 ก่อน + cache) — Anime-Waku/FairyAnime จาก ~20 วิ เหลือ ~4 วิ (ทั้งหมดจากกดเล่น ~6–7 วิ)
- Info: ตัวอักษรไทย (ชื่อเซิร์ฟเวอร์ `หลัก`) ใน monospace เคยเป็นกล่อง → fallback ไปฟอนต์ body
- ดีบักเวลา: `WAKU_HTTP_LOG=<file>` (fast path / browser / `resolve <src> '<server>' N ms` / `player: open … after N ms`),
  `RMP_HLS_TRACE=1` + `RMP_MPV_LOG=<file>` (ฝั่ง player) — `_transfer\run2.cmd` บนคอมทดสอบตั้งให้ครบ
- ชื่อตอน: ani.zip → Kitsu → Jikan (Jikan **ต่อไม่ได้เลย** จากเน็ตคอมทดสอบ แม้ IPv4) — เรื่องใหม่มาก (เช่น Dark Machine) ยังไม่มีชื่อตอนในฐานข้อมูลไหนเลย จะขึ้น "Episode N"

### รอบ 12 พ.ย. 2026 — ดาวน์โหลด, ข้าม OP/ED, ตอนถัดไปเร็ว, cookie Cloudflare (ทดสอบบน Windows แล้ว)

- **Cloudflare pass ลงดิสก์**: `<cache>/waku/cf_pass.json` (host, cookie, UA, เวลา; เก็บ 1 วัน) → เปิดแอปใหม่ไม่ต้องรอ browser
  (`cf passes from disk: 1` ใน http log, หน้าแรกของ Anime-Waku ~200 ms) — โดนปฏิเสธ (403/429/503) จะลบทิ้งเอง; 404 ไม่นับ
- **ตอนถัดไป**: เหลือ 150 วิสุดท้าย → `SourceManager::prefetch` หาลิงก์ตอนถัดไปไว้ (เก็บ 10 นาที, ใช้ครั้งเดียว) — กด Next แล้ว resolve = 0 วิ
- **AniSkip** (`meta/aniskip.rs`): ถามเมื่อรู้ความยาววิดีโอ (MAL id + ตอน); ปุ่ม "ข้ามเพลงเปิด/ปิด" ใน player (`PlayerView::set_skip`),
  ข้ามเองถ้าเปิด Settings › "ข้ามเพลงเปิด/ปิดอัตโนมัติ" (ค่าเริ่ม: เปิด)
- **ดาวน์โหลด** (`download.rs`, `dlpage.rs`): โฟลเดอร์ `Downloads\<เรื่อง>\` ข้าง exe (เปลี่ยนได้ด้วย `download_dir` ใน settings),
  `series.json` + `E05 ชื่อตอน.ts|.mp4`; HLS: เลือก variant คมสุด, AES-128 ถอดรหัส, ตัด header รูปปลอม, 4 ชิ้นพร้อมกัน, เขียน `.part` แล้ว rename
  แท็บ Downloads = งานที่กำลังโหลด + การ์ดเรื่อง (แบบหน้า Home) → เปิดดูตอนในเครื่อง เล่นได้ไม่ใช้เน็ต (source `local`,
  ความคืบหน้า/ดูต่อ บันทึกกลับไปที่ตอนของเว็บเดิม), ปุ่มถังขยะลบตอน
  ทดสอบ: Dark Machine EP5 (Anime-Waku 1080p) 389 MB ~5 วิ, TS ถูกต้อง, เล่นออฟไลน์ได้
- **หน้า Details ตามภาพอ้างอิง**: คอลัมน์ซีซั่นยาวเต็มรายการตอน, แถวตอน = ความยาว · วันฉาย, เรื่องย่อของตอน (ani.zip/Kitsu `summary`)
  แสดงใต้ตอนที่เลือก/ดูล่าสุด ตอนอื่นเป็น tooltip; ปุ่มดาวน์โหลดทุกตอน + "ดาวน์โหลดทั้งหมด"
- จับคู่ metadata: เรื่องที่ "ยังไม่ฉาย" โดนหักคะแนนเมื่อเว็บมีตอนแล้ว (เคยจับ Frieren เป็น 3rd Season → AniSkip หาไม่เจอ) และจับใหม่เอง

### รอบ 12 พ.ย. 2026 (บ่าย) — สถานะแหล่ง, ค้นหาด้วยภาพ, Discover, Discord, ลำดับการดู, Watch party

- **สถานะแหล่ง** (`srcpage.rs`, ข้อมูล `waku_core::srcstats`): ทุก list / search / detail / resolve / เล่น ถูกจับเวลา+ผล
  (60 ครั้งล่าสุดต่ออย่าง, `<cache>/waku/srcstats.json`) → การ์ดต่อแหล่ง: ค่ากลางเวลา, % สำเร็จ, กราฟแท่ง 30 ครั้ง, error ล่าสุด,
  ปุ่ม Test (ping + รายการล่าสุด + ค้นหา) — เปิดจากตัวเลข 5/5 ในแถบซ้าย หรือ Settings
  (ระวัง: `waku_core::health` คือของเดิม = ความน่าเชื่อถือของ video host, คนละอันกับ srcstats)
- **ค้นหาด้วยภาพ** (`scene.rs`, `meta/tracemoe.rs`): ลากรูปมาวาง / Ctrl+V / ปุ่มวางรูป / ลิงก์รูป → trace.moe → เรื่อง, ตอน, นาที, % ตรง
  → กด "หาแล้วเล่น" ค้นหาในทุกแหล่ง → หน้าเรื่องที่ตรง (AniList id เดียวกัน) มีปุ่ม "เล่น EP n จาก mm:ss" (`start_once`)
- **Browse › ค้นหาแบบละเอียด** (`discover.rs`, `anilist::discover`): แคตตาล็อก AniList กรองตามประเภท (หลายอัน), รูปแบบ (TV/Movie/OVA…),
  ปี, ฤดู, สถานะ, คะแนนขั้นต่ำ, ชื่อ; เรียงตามยอดนิยม/มาแรง/คะแนน/ใหม่/คนชอบ; 30 ต่อหน้า → กดการ์ด = ค้นหาในทุกแหล่ง
- **Discord Rich Presence** (`discord.rs`): IPC ตรง (`\\.\pipe\discord-ipc-N`), "Watching <เรื่อง> · EP · แหล่ง" + ปก + เวลาที่เหลือ,
  หยุดชั่วคราวแสดง "หยุดไว้" — ต้องใส่ Discord Application ID ใน Settings (สร้างที่ discord.com/developers, ชื่อแอป = ชื่อที่ขึ้น)
- **ลำดับการดู** (แท็บในหน้าเรื่อง, `order.rs`, `anilist::watch_order`): ไล่ relations PREQUEL/SEQUEL (+ side story / parent จากสายหลัก),
  สูงสุด 16 คำขอ, เรียงตามวันฉาย (`AnimeMeta.start`), แคช 7 วัน `<cache>/waku/order/<id>.json`; บอก "คุณอยู่ตรงนี้", ดูไปกี่ตอน, ยังไม่ฉาย,
  สวิตช์ "เฉพาะเนื้อเรื่องหลัก"; กด = ค้นหาในแหล่ง
- **Watch party** (`party.rs`, ปุ่ม Party ในแถบซ้าย): host เปิด TCP 47800, คนอื่น join ด้วย IP:port; JSON ทีละบรรทัด
  (hello / members / load / state / chat); ทุกคนควบคุมได้ (เล่น/หยุด/เลื่อน/เปลี่ยนตอน), host ส่งตำแหน่งทุก 3 วิ (คลาดเกิน 2 วิ → seek),
  ไม่ส่งอะไรระหว่างตัวเล่นกำลังเปิดไฟล์ (กันการ "หยุด" ทั้งห้อง), แชทขึ้นบนวิดีโอ; ไฟล์ดาวน์โหลดไม่ถูกแชร์ (คนอื่นเปิดไม่ได้)
  ข้ามเน็ตต้องใช้ VPN (Tailscale/ZeroTier/Radmin) หรือ forward port — ไม่มีเซิร์ฟเวอร์กลาง
- rmp `PlayerView`: `position()`, `seek_to()`, `set_paused()`, `go_to()`, `notice()`
- ทดสอบ: unit (srcstats, tracemoe parse, discord IPC กับ Discord จำลอง, party host+guest บน localhost), หน้าจอทั้งหมดใน Xvfb
  **ยังไม่ได้ทดสอบบน Windows**: build, Discord จริง, trace.moe จริง (บล็อกจาก cloud), party 2 เครื่อง, AniList Discover/ลำดับการดู (บล็อกจาก cloud)

### รอบ 12 พ.ย. 2026 (เย็น) — Watch party ข้ามเน็ตด้วยรหัสห้อง + อัปเดตจาก GitHub

**Watch party แบบรหัสห้อง** (`relay.rs`, ค่าเริ่มของหน้าต่าง Party; แบบ LAN/IP ยังอยู่เป็นอีกแท็บ)
- รหัส 8 ตัว Crockford base32 (`K7F3-9QMX`, ~40 bit; พิมพ์เล็ก/ไม่มีขีด/O↔0, I/L↔1 ก็ได้)
- PBKDF2-HMAC-SHA256 210 000 รอบ → 64 byte: 32 แรก = กุญแจ AES-256-GCM, ที่เหลือ = ชื่อ topic บน ntfy.sh
  (topic ไม่บอกรหัส, เดารหัสต้องเสีย PBKDF2 ทุกครั้ง → แม้แต่ ntfy.sh เองก็ถอดไม่ได้ในทางปฏิบัติ)
- ทุกข้อความ: nonce สุ่ม 96 bit + AES-GCM ของ `{from, seq, at, w}` → base64; ผิดกุญแจ/ถูกแก้ = ทิ้ง; seq ต่อผู้ส่ง + เวลา ±10 นาที กัน replay
- ntfy.sh จำกัด ~250 ข้อความ/วัน/IP (burst 60 แล้ว 1 ต่อ 5 วิ) → ส่งเฉพาะเหตุการณ์ (เล่น/หยุด/เลื่อน/ตอน/แชท/เข้า-ออก),
  รวมข้อความที่มาติดกัน (ตำแหน่งล่าสุดเท่านั้น), ห่างกันอย่างน้อย 1 วิ, host เช็กตำแหน่งทุก 2 นาที (LAN ยัง 3 วิ); โดน 429 → แจ้งในแชท + รอแล้วลองใหม่
- รับ: `GET <server>/<topic>/json` (stream, keepalive ~45 วิ, timeout 100 วิ → ต่อใหม่ด้วย `since=<id ล่าสุด>` ไม่พลาดข้อความ)
- เปลี่ยนเซิร์ฟเวอร์ได้ (`party_server` ใน settings.json; ntfy self-host ก็ใช้ได้)
- ทดสอบ: crypto/รหัส/replay/การรวมข้อความ + 2 คนผ่าน ntfy จำลองบน localhost (ห้องอื่นไม่ได้ยิน, เซิร์ฟเวอร์เห็นแต่ข้อความเข้ารหัส)

**อัปเดตจาก GitHub** (`update.rs`, Settings › อัปเดต; repo ค่าเริ่ม `sapph1rer/waku-app` — เปลี่ยนได้)
- แอป: `releases/latest` → zip ที่ชื่อมี windows/x64 + **ต้องมี `<zip>.sha256`** (ไม่มี = ไม่ติดตั้ง) → แตก zip →
  ไฟล์เดิมเปลี่ยนชื่อเป็น `<ชื่อ>.old-<เวลา>` (Windows ยอมให้เปลี่ยนชื่อ exe/dll ที่กำลังรัน) → คัดลอกของใหม่ → "เปิดใหม่ตอนนี้";
  `.old-*` ลบตอนเปิดครั้งถัดไป; ตัวที่รันจากโฟลเดอร์ `target` (cargo) จะไม่ถูกแทน แค่ให้ลิงก์หน้า Release
- ตรวจเองหลังเปิดแอป 15 วิ (ปิดได้), แจ้งเตือนเมื่อมีเวอร์ชันใหม่
- กฎดึงข้อมูล: `crates/waku-core/sources.default.toml` บน main มี `revision = N` → ถ้าใหม่กว่า: ไฟล์ในเครื่องที่ไม่ได้แก้เอง
  (เทียบ hash กับที่แอปเขียนไว้ใน `sources.applied`) อัปเดตเอง + โหลดแหล่งใหม่; ถ้าแก้เอง → รอให้กด "ใช้รุ่นใหม่ (สำรองไฟล์เดิม)"
  → **แก้กฎแล้วต้องเพิ่มเลข revision ก่อน push**
- ออกเวอร์ชัน: แก้ `version` ใน `crates/waku-app/Cargo.toml` แล้ว
  `powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1 -Notes "..."` (build → `dist\waku-windows-x64.zip` + `.sha256` → `gh release create`)

### ผลทดสอบบน Windows 12 พ.ย. 2026 (หลังออก v0.1.0)

- Watch party ผ่าน ntfy.sh จริง (2 หน้าต่าง, รีโมต `party host|join CODE|chat|pause|play|seek|leave` ใน dev):
  เข้าห้อง/รายชื่อ/แชทสองทาง ✅, A เล่น → B เปิดตอนเดียวกันเอง (ห่าง ~1 วิ) ✅, หยุด ✅, เลื่อนไป 400 วิ + เล่น ✅
  **บั๊กที่เจอและแก้แล้ว:** ระหว่างดูวิดีโอ `ui()` return ก่อนถึง `party_ui` → ปาร์ตี้ไม่ sync ตอนเล่น (v0.1.0 มีบั๊กนี้)
- สถานะแหล่ง ✅ (ตัวเลขจริง), Discover/AniList ✅, ลำดับการดู ✅ (Frieren 6 ภาค) — แก้ป้าย "ดูแล้ว N ตอน" เป็น "ดูถึงตอน N"
  (history เก็บแค่ตอนล่าสุดต่อเรื่อง), ค้นหาด้วยภาพ ✅ (เฟรม Frieren → EP 3 นาที 4:07 98%) → หาแล้วเล่น → เริ่มที่ 247.7 วิ ✅
  (`party scene <ไฟล์รูป>` ใน dev = ค้นหาด้วยไฟล์)
- กฎดึงข้อมูล revision 1 บน GitHub: เครื่องทดสอบขึ้น "รอให้กดใช้" เพราะไฟล์เดิมสร้างจากรุ่นก่อนมี revision (ตามที่ออกแบบ)

### รอบ 12 พ.ย. 2026 (ดึก) — ความเสถียร + UI จากที่ผู้ใช้ลองจริง

- **ปก "ดูต่อ" ไม่ขึ้น**: บาง source (anifume) ไม่มีรูปในการ์ดที่บันทึกลง history (`poster: null`) และชื่อเป็นไทยล้วน → จับคู่ AniList ไม่ได้
  แก้: `Library::fill_poster` (หน้าเรื่องโหลดมาเมื่อไรก็เติมรูปให้ history/My List), `find_poster` (การ์ดไม่มีรูป → ใช้ detail cache หรือโหลดหน้าเรื่องเบื้องหลังครั้งเดียว),
  `watched()` ไม่ทิ้งรูปเดิม, รูปที่โหลดพลาดลองใหม่ได้ 3 ครั้ง (8/16/24 วิ) แทนการพังถาวร, ปก AniList โหลดไม่ได้ → ใช้รูปของเว็บ
- ปุ่ม ✕ บนการ์ด "ดูต่อ" โผล่เฉพาะตอนเอาเมาส์ชี้
- **บั๊ก hero หน้า Home**: กดลูกศรแล้วข้อความไม่ขึ้นจนกว่าเมาส์จะออก เพราะตอน hover ตัวนับเวลาสไลด์ถูกหยุด และ fade ใช้ตัวนับเดียวกัน → แยก `hero_shown` เป็นนาฬิกาของ fade
- หน้าเรื่อง: hero สูงขั้นต่ำ 330 (เดิม 440) — ไม่มีช่องว่างใต้ปุ่ม
- **ค้นหาด้วยภาพ** ย้ายเป็นไอคอนในช่องค้นหา (ขวา) → dialog (`egui::Modal`): ลากไฟล์มาวาง (ลากรูปเข้าหน้าค้นหาเฉยๆ ก็เปิด dialog เอง), Ctrl+V รูป
  (หรือวางลิงก์), คลิกเลือกไฟล์ (`rfd`, Windows), หรือพิมพ์ลิงก์ — ผลลัพธ์อยู่ใน dialog
- **เปิดแอป**: ตรวจอัปเดตก่อน (สูงสุด 4 วิ, ข้อความ "กำลังตรวจหาอัปเดต…") แล้วค่อย "กำลังเชื่อมต่อแหล่ง" — ถ้ามีเวอร์ชันใหม่ หน้าเปิดแอปถาม
  "อัปเดตเลย / ไว้ทีหลัง" (อัปเดตเลย = ดาวน์โหลดพร้อมแถบความคืบหน้าแล้วเปิดตัวใหม่เอง); build จากโฟลเดอร์ `target` ไม่ถาม
- **ชื่อตอน/ชื่อซีซั่น**: จับคู่ AniList ได้มากขึ้น — ชื่อภาษาอังกฤษ/โรมาจิในเรื่องย่อภาษาไทย (`meta::latin_names`), ตัวอักษรหน้าตาเหมือนกัน
  (AniList เขียน "Juuоu" ด้วย о ซีริลลิก → `text::fold_lookalike`), ค้นด้วยคำที่ยาวที่สุดแล้วเทียบกับชื่อเต็ม; ผลเก่าที่หาไม่เจอถูกถามใหม่ (`EPS_REV` 3)
  คอลัมน์ซีซั่นแสดงชื่อซีซั่นจาก AniList (ซีซั่น n = ภาค TV ลำดับที่ n ของ franchise ในลำดับการดู)
- ความเสถียร: panic ทุก thread เขียนลง `%APPDATA%\waku\crash.log` (มี backtrace), งานเบื้องหลังที่ panic ไม่ทำให้แอปตาย (`Msg::Crashed` → แจ้งเตือน),
  worker metadata ไม่ตายเพราะเรื่องเดียว
- dev: `ctrl KEY`, `hoverfile SECS`, `drop PATH` (จำลองลากไฟล์มาวาง); `page` ต้องเป็นตัวใหญ่ (`page Search`)

### รอบ 13 พ.ย. 2026 — แหล่ง 18+, Watch party ตรงกันขึ้น, แชทจาง (ยังไม่ได้ทดสอบบน Windows)

- **แหล่ง 18+ (hentai)**: `SiteProfile.adult`, `Settings.adult_sources` (ปิดเป็นค่าเริ่มต้น) — ตั้งค่า › แหล่งอนิเมะ มีสวิตช์ + ถามอายุ 18+ ก่อนเปิด
  - ปิด: `SourceManager::load()` ไม่โหลดแหล่งที่ `adult = true` → หน้าเปิดแอปเชื่อมต่อแค่แหล่งหลัก, ค้นหา/หน้า Home ไม่มี, การ์ดใน ดูต่อ/My List/ประวัติ ถูกซ่อน (`shown_source`)
  - เปิด: โหลดเพิ่ม 3 แหล่ง `akih` (aki-h.com), `alphahen` (alpha-hen.com), `animeh` (anime-h.net) — แถบ featured บน Home ไม่ใช้แหล่ง 18+ แทนแหล่งหลัก
  - sources.toml ที่แก้มือ (ไม่ได้อัปเดตกฎอัตโนมัติ) ยังได้ 3 แหล่งนี้: `load_all` เติมจากค่าเริ่มต้นถ้าไม่มี id นั้น; กฎ revision 2
  - **selector ยังไม่ได้ตรวจ** (cloud เข้าเว็บพวกนี้ไม่ได้) — aki-h ใช้ path ที่เห็น (`/episode/<slug>/`, `/latest/`), อีกสองเว็บใช้ heuristic → ต้องทดสอบบนคอม
- **Watch party ตรงกันขึ้น** (ฝั่งที่ join ช้ากว่าหัวห้อง):
  - สาเหตุ: ntfy ส่งช้า ~0.3–1.5 วิ + คิวส่งรอ ≤1 วิ แต่ผู้รับบวกแค่ 0.2 วิ, ตอนเปิดตอนใหม่ใช้เวลาโหลดหลายวิ แล้วแก้เมื่อห่าง >2 วิ ใน heartbeat ทุก 2 นาทีเท่านั้น
  - ใหม่: นาฬิกาของห้อง = นาฬิกาหัวห้อง; guest ส่ง `Ping` 3 ครั้งตอนเข้า (แล้วทุก 20 นาที / LAN ทุกนาที), หัวห้องตอบ `Pong` แบบ NTP (เวลารับ/ส่ง, เวลาถูกเขียนตอนออกจริงใน `send_loop`)
    → offset จากรอบที่ RTT สั้นสุด; `State`/`Load` มี `at` (เวลาที่อ่านตำแหน่ง, นาฬิกาห้อง) → ผู้รับรู้ว่าห้องอยู่ที่ไหน *ตอนนี้*
  - `party_catch_up` ทุก 200 ms: ห่าง >2 วิ → seek (+0.5 วิเผื่อเวลา seek), 0.08–2 วิ → เร่ง/ชะลอ ±10% (`PlayerView::set_sync_rate` ใน rmp — ป้ายความเร็วยังเป็นของผู้ใช้), ≤0.08 วิ → ปกติ
  - ใครส่งล่าสุดคือคนที่ห้องตาม (`Reference.mine`); รุ่นเก่า (ไม่มี `at`) ยังคุยกันได้ (`#[serde(default)]`, ถือว่าเดินทาง 0.3 วิ)
  - แผงปาร์ตี้บอก "ตรงกับห้องแล้ว / กำลังตามห้อง — ช้ากว่า x วิ"
- **แชทปาร์ตี้มุมขวาล่าง** จาง (35%) จนกว่าเมาส์จะชี้หรือกำลังพิมพ์
- dev ที่ยังไม่มี: คำสั่งจำลองความหน่วงของ ntfy — ทดสอบจริงด้วย 2 หน้าต่าง (`run_a`/`run_b`) ผ่าน ntfy.sh

### ยังไม่ได้ทดสอบ / ปัญหาที่รู้

- Anime-Waku (Psyren, Dark Machine), FairyAnime: เล่นจากตัวเล่นหลัก 1080p บน Windows ได้แล้ว; ปุ่ม ✕ "ดูต่อ" บน Windows และเมนูความละเอียดยังไม่ได้กดทดสอบผ่านรีโมต
- `waku-browser` sniff ผ่านการทดสอบ (fairy: เจอลิงก์ HLS ของ tonytonychopper) แต่ยังไม่ได้ลองเล่นลิงก์นั้นใน player จนจบ
- anifume `.mp4` resolve ได้ แต่ยังไม่ได้ probe ใน mpv (ลิงก์มีวันหมดอายุ `e=` ~1–2 ชม. → แอป resolve ใหม่ทุกครั้งที่กดเล่น)
- **abyss** (googleapis chunk) ไม่รองรับ — ไฟล์ตอบ 403 ถ้าไม่ได้ขอเป็นชิ้นแบบที่ player ของมันทำ → ข้ามไปเซิร์ฟเวอร์อื่น
- **สระ/วรรณยุกต์ไทย**: egui ยังไม่มี text shaping บางคำสระบน+วรรณยุกต์อาจซ้อนไม่สวย
- โปสเตอร์จาก host ที่ติด Cloudflare (anime-waku `wp-content/uploads`) โหลดผ่าน HTTP ปกติได้ในตอนทดสอบ ถ้าวันไหนโดนบล็อกรูปจะเป็น placeholder

## Disclaimer

สำหรับใช้งานส่วนตัว/การศึกษา แอปไม่ได้โฮสต์หรือเก็บไฟล์วิดีโอ เนื้อหาทั้งหมดมาจากเว็บไซต์ภายนอก

## License

MIT
