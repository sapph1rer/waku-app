# 🍙 Waku (desktop)

แอปดูอนิเมะซับไทยบนเดสก์ท็อป เขียนด้วย Rust + egui — ต่อยอดจาก [Waku-cli](https://github.com/sapph1rer/Waku-cli)
ดึงจาก **หลายแหล่งพร้อมกัน** ถ้าแหล่งหนึ่งช้า/ล่ม/เซิร์ฟเวอร์เสีย จะสลับไปหาตอนเดียวกันจากแหล่งอื่นให้อัตโนมัติ
หน้าตา: sidebar ไอคอนด้านซ้าย, hero ภาพพื้นหลังเฟดเข้าดำ, แท็บ Episodes / Details / Other Sources / More Like This,
คอลัมน์ซีซั่น + การ์ดตอนแบบมี thumbnail และ progress bar — UI ภาษา **English (ค่าเริ่มต้น) / ไทย** เปลี่ยนได้ใน Settings

## ฟีเจอร์

- **Home** — hero แนะนำ (หมุนอัตโนมัติ), แถว Continue Watching, แถว "New on <แหล่ง>" ของทุกแหล่ง
- **Browse** — grid ของแต่ละแหล่ง สลับด้วย chip ที่มีจุดสถานะ เขียว/เหลือง/แดง + โหลดเพิ่ม
- **ค้นหาทุกแหล่งพร้อมกัน** — ผลขึ้นทีละแหล่งตามที่ตอบกลับ, fuzzy search จาก Waku-cli (`re zero` / `rezero` / ชื่อไทย) + index จาก sitemap
- **หน้าเรื่อง** — poster, เรื่องย่อ, แท็ก, รายการตอน (แยกซีซั่น, เรียงกลับได้), ปุ่ม "ดูต่อ: ตอนที่ N", รายการของฉัน
- **แหล่งอื่น** — หาเรื่องเดียวกันจากแหล่งอื่น พร้อม % ความเหมือนของชื่อ (เช็กเลขซีซั่นด้วย กันจับผิดภาค)
- **เล่นด้วย mpv** แบบเดียวกับ Waku-cli: sniff สตรีมตรง (.m3u8/.mp4/doodee `.txt`), เลือกคุณภาพ, ลอง Referer หลายแบบ
- **Failover อัตโนมัติ**: ทุกเซิร์ฟเวอร์ของแหล่งนี้ → (เปิด/ปิดได้) หาตอนเดียวกันในแหล่งอื่นตามลำดับที่ตั้งไว้ → เล่นต่อ
- **เลือกเซิร์ฟเวอร์เอง** — ปุ่มขวาของแต่ละตอน บอกชนิดสตรีม (ตรง / yt-dlp / ทางสุดท้าย) และความละเอียดที่มี
- **ตั้งค่า** — ภาษา (English/ไทย), path ของ player, คุณภาพ, ขนาด UI, ลำดับแหล่ง, ตรวจสถานะ, **ปุ่มทดสอบแหล่ง** (บอกว่าพังขั้นไหน)

## Build (Windows)

```powershell
cargo build --release
```

ไฟล์อยู่ที่ `target\release\waku-app.exe` วางโครงแบบนี้:

```
waku-app.exe
mpv\mpv.exe          ← ต้องมี (หรืออยู่ใน PATH / ตั้ง path ในหน้าตั้งค่า)
mpv\yt-dlp.exe       ← แนะนำ (ok.ru, dailymotion ฯลฯ)
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

| id | เว็บ | สถานะ |
|---|---|---|
| `waku` | anime-waku.com | DooPlay — selector ยืนยันจาก Waku-cli |
| `sugoi` | anime-sugoi.net | WordPress + Elementor, หน้าเรื่อง `/<slug>/` — ยังไม่ได้ยืนยัน player |
| `anifume`, `animeruka`, `fairy`, `hareshi` | | **heuristic** — ยังไม่ได้ตรวจ markup จริง |

ตอนพัฒนาผมดึงหน้าเว็บ 4 แหล่งหลังไม่ได้ (บล็อกจากต่างประเทศ) เลยตั้งเป็นโหมด heuristic ซึ่งเดาเอง:
การ์ด = ลิงก์ในเว็บเดียวกันที่มีรูป, ตอน = ลิงก์ที่มี `ตอนที่` / `EP` / `-ep-`, player = DooPlay / iframe / `<option value>` (รวม base64) / `data-src`…
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
  manager.rs       SourceManager: health, search_all, alternatives, match_episode, streams, diagnose
  player.rs        เรียก mpv/vlc (+ yt-dlp hook), ตรวจ exit code 2 เพื่อ failover
  store.rs         settings.json / library.json (รายการของฉัน, ประวัติ)
crates/waku-app    GUI egui 0.36
  app.rs           state, background jobs (thread + channel), ลำดับการเล่น + failover
  i18n.rs          tr("English", "ไทย") / trf!(…) — เพิ่มภาษาใหม่ = เพิ่ม variant + argument
  pages.rs         Home / Browse / Search / My List / History / Settings
  detail.rs        หน้าเรื่อง: hero, แท็บ, คอลัมน์ซีซั่น, การ์ดตอน, ตัวเลือกเซิร์ฟเวอร์
  widgets.rs       backdrop + gradient, การ์ด poster/ตอน, แท็บ, sidebar, ปุ่ม
  theme.rs         สี, ฟอนต์ไทย/ญี่ปุ่น, icon
```

ถ้าเว็บไหนไม่ใช่ WordPress (เช่นเป็น SPA ที่โหลดข้อมูลผ่าน JSON API) ให้เขียน struct ใหม่ที่ `impl Source`
แล้วเพิ่มใน `SourceManager::from_profiles` (มีคอมเมนต์จุดที่ต้องใส่ไว้แล้ว) — UI และ failover ใช้ได้ทันที

## ข้อจำกัดที่รู้อยู่

- **Cloudflare challenge**: ถ้าเว็บเปิด "Under attack" reqwest จะได้ 403/503 (แอปบอกว่าเป็น Cloudflare) — ตอนนี้ยังไม่มีตัวแก้ challenge, failover จะข้ามไปแหล่งอื่นแทน
- **สระ/วรรณยุกต์ไทย**: egui ยังไม่มี text shaping บางคำสระบน+วรรณยุกต์อาจซ้อนไม่สวย ฟอนต์ที่ออกแบบให้ตำแหน่งตายตัว (เช่น Tahoma) จะดีกว่า
- วิดีโอเล่นใน **mpv หน้าต่างแยก** (เหมือน Waku-cli) — ถ้าอยากฝังในแอป ทางต่อไปคือ libmpv render API + egui texture
- ประวัติบันทึกตอนที่ "เริ่มเล่น" (mpv ไม่บอกเวลาที่ดูถึง) — ต่อยอดได้ด้วย `--watch-later` / IPC ของ mpv

## Disclaimer

สำหรับใช้งานส่วนตัว/การศึกษา แอปไม่ได้โฮสต์หรือเก็บไฟล์วิดีโอ เนื้อหาทั้งหมดมาจากเว็บไซต์ภายนอก

## License

MIT
