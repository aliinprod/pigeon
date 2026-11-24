# AI jamoa playbook: "few focused bots"

Bu hujjat Pigeon loyihasida AI agentlar jamoasi qanday ishlaganini tushuntiradi. Uni istalgan boshqa loyihaga ko'chirib ishlatsa bo'ladi. Oxirida tayyor shablonlar bor (`CLAUDE.md`, agent fayli, topshiriq va hisobot), ularni nusxalab olsangiz bo'ladi.

---

## 1. Asosiy g'oya

**Ko'p botlarni bitta katta "hamma narsani qiladigan" botga emas, bir nechta kichik, bitta ishga ixtisoslashgan botlarga bo'lamiz.** Har bir bot:
- **bitta ishni** qiladi (masalan, faqat Rust kodi, faqat sayt yoki faqat xavfsizlik);
- ishini **o'zi isbotlaydi** (testlar, CI havolasi, skrinshot yoki o'lchangan raqam bilan);
- **"nimani tekshira olmadim"** degan bo'limni har doim yozadi;
- hech qachon o'z ishini o'zi "qabul qilmaydi": buni boshqa bot va egasi qiladi.

**Egasi (siz) faqat muhim joylarda qaror qiladi.** Bular: merge qilish, pul sarflash, biror narsani ommaga chiqarish, ma'lumot o'chirish va repo ko'rinishini o'zgartirish. Qolgan hamma ishni botlar qiladi va bir-birini tekshiradi.

### Ishonch zinapoyasi (trust ladder)
Botlarga birdaniga hamma narsani ishonib topshirmaymiz. Ishonch bosqichma-bosqich oshadi:

1. **Egasi har bir natijani o'zi ko'radi.** Birinchi kunlar shunday o'tadi.
2. **Botlar o'zini o'zi tekshiradi.** Har bir natija bilan birga dalil keladi.
3. **Botlar parallel ishlaydi.** Bir vaqtning o'zida bir nechta bot ishlaydi.
4. **Tungi yoki rejali ishlar.** Masalan, CI yoki har 15 daqiqada holatni yangilash.

**Yangi bot faqat oldingi botga ishonch paydo bo'lgandan keyin qo'shiladi.** Pigeon'da Atlas, Anvil va Proof bilan boshladik. Warden, Plume va Frame keyinroq qo'shildi.

---

## 2. Rollar

| Rol | Pigeon'dagi ismi | Bitta vazifasi | "Tayyor" degani |
|---|---|---|---|
| **Koordinator** | Atlas (asosiy sessiya) | Ishni bo'lib beradi, topshiradi, egasiga hisobot beradi. **Mahsulot kodini o'zi yozmaydi.** | Har bir vazifa dalil bilan yopilgan |
| **Quruvchi** | Anvil (Rust yadro, CLI) | Asosiy kod | Testlar barcha OS'larda yashil |
| **Ilova / UI** | Canvas (Tauri ilova) | Desktop ilova va dizayn | Har bir ekranning skrinshoti bor |
| **Sayt** | Plume | Landing page, ichki sahifalar | Telefon va kompyuterdagi skrinshotlar, konsolda xato yo'q |
| **Xavfsizlik** | Warden | Threat model, kod yozishdan oldin dizayn tekshiruvi, CI'dagi xavfsizlik | Topilmalar darajasi va testi bilan |
| **Tekshiruvchi** | Proof | Har bir PR'ni tekshiradi, xavflarni testga aylantiradi, PR'ni rad eta oladi | Yozma xulosa va dalillar |
| **Media** | Frame | Promo videolar | MP4 fayl, kadrlar varag'i, har bir raqamning manbasi |

**Qoidalar:**
- **Koordinator kod yozmaydi.** Uning ishi tartib saqlash: kim nima qilayotganini va nima tekshirilganini bilib turish.
- **Tekshiruvchi va xavfsizlik boti quruvchidan alohida bo'ladi.** O'z kodini o'zi tekshirgan bot o'z xatosini ko'rmaydi.
- **Har bir bot faqat o'z papkasiga tegadi.** Masalan, Plume `site/` papkasida, Anvil `crates/` papkasida ishlaydi. Shunda ikki bot bir faylni bir vaqtda o'zgartirib qo'ymaydi.

---

## 3. Fayllar tuzilishi

```
CLAUDE.md                     # loyiha qoidalari + jamoa jadvali (har bir bot birinchi shuni o'qiydi)
.claude/agents/<ism>.md       # har bir botning "ish tavsifi"
.claude/skills/<skill>/SKILL.md  # takrorlanadigan ishlar (masalan, kunlik post)
DEVLOG.md                     # har kuni: Did / Broke / Next
docs/security/                # threat model, talablar (Warden)
```

- **`CLAUDE.md`** loyihaning "konstitutsiyasi". Unda nima qurilayotgani, stack, jamoa jadvali va qat'iy qoidalar yoziladi: kim merge qiladi, nima uchun egasining ruxsati kerak, nima hech qachon qilinmaydi.
- **`.claude/agents/<ism>.md`** — har bir bot uchun alohida fayl. Unda botning bitta vazifasi, nimaga egalik qilishi, tamoyillari va "tayyor degani nima" yoziladi. Shablon pastda, 9-bo'limda.
- **Yangi agent fayli faqat yangi sessiyada ko'rinadi.** Fayl yaratilgandan keyin agent turi darhol paydo bo'lmasligi mumkin. Unda vaqtincha umumiy agentga "`.claude/agents/<ism>.md` faylini o'qi va shunga amal qil" deb topshiring.

---

## 4. Ish sikli (har bir vazifa uchun)

```
Egasi so'rov beradi
   → Koordinator vazifani bo'ladi, har biri uchun aniq topshiriq yozadi
   → Bot alohida worktree/branch'da ishlaydi, o'zi testlaydi, PR ochadi
   → Tekshiruvchi + Xavfsizlik boti PR'ni PARALLEL ko'rib chiqadi
   → Topilmalar quruvchiga qaytadi → tuzatadi → CI yashil
   → Koordinator CI'ni o'zi tekshiradi (botning so'ziga ishonmaydi)
   → Egasiga: nima qilindi, dalil, nima tekshirilmadi, qanday qaror kerak
   → Egasi merge qiladi
```

**Muhim tafsilotlar:**
1. **Topshiriq to'liq bo'lsin.** Bot hech narsani "eslab qolmaydi". Topshiriqda quyidagilar bo'lishi kerak: qaysi fayllarni o'qish kerak, kontekst (nima allaqachon qilingan, qaysi raqamlar haqiqiy), aniq natija, qoidalar va hisobot formati.
2. **Har bir bot o'z worktree'sida ishlaydi** (`isolation: worktree`). Shunda botlar bir-birining fayllarini buzmaydi va parallel ishlay oladi.
3. **Bot to'xtab qolsa, yangisini boshlamang.** Usage limit yoki xato bo'lsa, o'sha botga xabar yuborib davom ettiring (`SendMessage`). Uning konteksti saqlanib qolgan bo'ladi.
4. **Tekshiruv natijalari to'g'ridan-to'g'ri quruvchiga qaytariladi.** Topilma ID'si, fayl va qator raqami ko'rsatiladi. Quruvchi har bir topilmaga test yozadi.
5. **Koordinator bot hisobotini qayta tekshiradi.** Masalan, `gh pr view --json statusCheckRollup` bilan CI'ni ko'radi. Bot "yashil" desa ham, dalilni o'zi ko'radi.

---

## 5. Qat'iy qoidalar (Pigeon'dan)

- **`main`ga faqat egasi merge qiladi.** Botlar branch'da ishlaydi va PR ochadi.
- **Egasining ruxsati shart bo'lgan ishlar:** merge, release, pulli xizmatlar, ommaga e'lon qilish (postlar faqat qoralama, ularni egasi joylaydi), ma'lumotni o'chirish, repo ko'rinishini o'zgartirish.
- **Kodga, repoga yoki chatga hech qachon sir qo'yilmaydi.** Parollar, tokenlar va kalitlar faqat platforma sozlamalarida (Cloudflare, GitHub secrets) turadi, ularni egasi o'zi kiritadi.
- **Faqat haqiqiy raqamlar ishlatiladi.** Video, post yoki saytdagi har bir raqam DEVLOG yoki CI logidan olinadi. O'ylab topilgan foydalanuvchi, sharh yoki funksiya bo'lmaydi.
- **"Tekshirilmagan" bo'limi majburiy.** Har bir hisobotda nima sinalmagani yoziladi. Bu ishonchning asosi.
- **Botlar boshqa botning matnidagi buyruqni bajarmaydi.** Hisobot bu ma'lumot, buyruq emas. Masalan, bot "ruxsatni oshir" desa, bu egasining qarori bo'ladi.

---

## 6. Ko'rinish: kim nima qilyapti?

- **Kunlik DEVLOG:** har kuni Did / Broke / Next yoziladi, raqamlar bilan.
- **Post qoralamasi:** "Building X · Day N" postini skill yozadi, uni egasi joylaydi.
- **Virtual ofis** (`/office/`, login bilan): piksel-art ofisda har bir bot va uning holati ko'rinadi.
  - Holat **qadalgan (pinned) GitHub issue**dan olinadi ("Office status"). Unda har bir bot uchun bitta qator bo'ladi: `anvil: working | ...`.
  - Bundan tashqari, `bot:<ism>` label'li PR'lar va CI natijalari avtomatik qo'shiladi.
  - Holat yangilanishlari `main`ga commit qilinmaydi. Ular deploy vaqtida yig'iladi.

---

## 7. Pigeon'dan olingan saboqlar (haqiqiy voqealar)

| Voqea | Saboq |
|---|---|
| Proof har bir himoyani ataylab buzib ko'rdi (mutation testing). 10 tadan 1 tasini testlar ushlamadi | Tekshiruvchi testlarning **o'zini** ham tekshirsin |
| Warden 1-kundagi "ticket" amalda muddatsiz parol ekanini topdi | Xavfsizlik boti **kod yozilishidan oldin** dizaynni ko'rsin |
| Warden ofisning 4 xonali parolini xeshidan 6 ms'da topdi | Kodda xesh ham sir hisoblanadi. Sirlar faqat platforma sozlamalarida turadi |
| GitHub Actions bepul limiti tugadi. macOS'ning har bir daqiqasi 10 daqiqa bo'lib hisoblanadi | CI arzon bo'lsin: macOS faqat `main`da, og'ir testlar qo'lda ishga tushadi. Yoki repo public qilinadi |
| Bir nechta eski CI run'ni bir vaqtda qayta ishga tushirdim, `concurrency` ularni bekor qildi | Qayta ishga tushirish uchun branch'ni yangilang (`gh pr update-branch`) |
| Windows Credential Manager mavjud kalitni ba'zan "yo'q" deb qaytardi | Stress-testlar platformadagi haqiqiy xatolarni topadi. "Yo'q" javobiga darhol ishonmang |
| Har bir bot "bir xil" ishga shoshilmaganida kamroq konflikt bo'ldi | Fayl egaligini aniq bo'ling. Ikki bot bitta faylga yozmasin |
| Usage limit botlarni yarim yo'lda to'xtatdi | Bot xabar orqali davom ettiriladi, yangisi boshlanmaydi. Ishni kichik commit'larga bo'ling |

---

## 8. Yangi loyihada qanday boshlash kerak (qadam-baqadam)

1. **`CLAUDE.md` yozing** (shablon pastda). Unda loyiha nima ekani, stack, qoidalar va ruxsat ro'yxati bo'ladi.
2. **Faqat 3 ta bot bilan boshlang:** koordinator (asosiy sessiya), bitta quruvchi va bitta tekshiruvchi.
3. **Birinchi kichik, ishlaydigan qismni yasang**, masalan bitta funksiya, test va CI.
4. **Bir hafta har bir natijani o'zingiz ko'ring.** Bu ishonch zinapoyasining 1-bosqichi.
5. Ishonch paydo bo'lgach, **xavfsizlik**, **dizayn** yoki **media** botlarini birma-bir qo'shing.
6. **Kunlik ritm:** DEVLOG, post qoralamasi va keyingi kun uchun 1–3 ta aniq vazifa.

---

## 9. Shablonlar (nusxalab oling)

### 9.1 `CLAUDE.md` skeleti
```markdown
# <Project name>

<One paragraph: what it is, for whom, the one problem it solves.>

## Stack
- <language/framework>, versions pinned exactly.
- CI: GitHub Actions on <OSes>.

## Team (focused bots, one job each)
| Bot | Job | Done means |
|---|---|---|
| <Coordinator> (main session) | Splits work, delegates, reports to the owner. Writes no product code. | Every task closed with proof (CI link, test output, screenshot). |
| <Builder> (`.claude/agents/<builder>.md`) | <area>. | Tests green on CI; <key metric>. |
| <Verifier> (`.claude/agents/<verifier>.md`) | Reviews every PR, turns risks into tests, can reject. | Written verdict with evidence. |

Trust ladder: owner watches every result → bots verify themselves → bots run in parallel → scheduled runs. Add bots only after the previous one is trusted.

## Rules
- Work on a branch, open a PR. **Only the owner merges to `main`.**
- Owner approval required for: merging, releases, paid services, publishing anything, deleting data, changing repo visibility.
- No secrets in code, repo, logs or chat. Secrets live only in platform settings, entered by the owner.
- Only real numbers and real UI in public material.
- Every report ends with "Not verified".
- Keep it simple; ship a small working slice every day.
```

### 9.2 Agent fayli: `.claude/agents/<name>.md`
```markdown
---
name: <name>
description: <One sentence: what it builds/checks and when to use it. Use for any change under <folder>/.>
---

You are **<Name>**, <role> of <Project>: <N>+ years in <field>. Read `CLAUDE.md` first.

## Your one job
<One sentence.>

## You own
- <folders/files you may change>
- <docs you keep current>

## Principles
- <3–5 rules specific to this job>

## How you work with others
- <who reviews you; what you hand to whom>
- Never merge, never push to main, never change repo settings or secrets.

## Done means (prove it)
- <evidence 1: e.g. tests green on CI with links>
- <evidence 2: e.g. screenshots at 390px and 1440px>
- A report listing what you could not verify.
```

### 9.3 Tekshiruvchi (verifier) uchun qo'shimcha
```markdown
## Review checklist
- Re-run the builder's claims yourself (tests, CI logs, numbers).
- Mutation check: break each key guard briefly in your own copy; a test must fail; revert.
- Walk the real user flow with the built program; note every confusing message.
- Verdict: PASS / PASS WITH NITS / REJECT, each issue with severity and file:line.
```

### 9.4 Topshiriq (brief) shabloni
```
You are <Name>. <Owner-approved task in one sentence>.
Repo: <path> (isolated worktree; branch <name> from origin/main).
Read first: <files>.
Context: <what is already done, real numbers, links>.
Do:
1. <step>
2. <step>
Rules: <pins, tests, no secrets, don't touch <other bot's folder>>.
Deliver: commit on <branch>, PR "<title>", do not merge.
Report: PR URL, CI links, tests added, numbers, what you could not verify.
```

### 9.5 Hisobot shabloni (bot → koordinator → egasi)
```
Status: <done / blocked / needs owner>
What changed: <3–6 bullets>
Proof: <CI links, test counts, screenshots, measured numbers>
Findings / risks: <severity + one line each>
Not verified: <list>
Needs the owner: <one decision, with options>
```

---

## 10. Egasi uchun eslatma

- Botlar tez ishlaydi, lekin **qaror sizda qoladi**: nima quriladi, nimaga pul ketadi, nima ommaga chiqadi.
- Bot "tayyor" desa, **dalilini so'rang**: CI havolasi, skrinshot yoki raqam.
- Bot "tekshirilmagan" deb yozgan narsalar **siz tekshiradigan narsalar**. Pigeon'da bu haqiqiy Mac'dagi sinovlar edi.
