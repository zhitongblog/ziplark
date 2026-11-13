// App Store Connect metadata for Ziplark (macOS).
// Subtitle <= 30 chars; keywords <= 100 chars.

export const appInfoLocalizations = {
  'zh-Hans': {
    name: 'Ziplark',
    subtitle: '全格式解压缩：RAR、7z、ZIP、tar',
    privacyPolicyUrl: 'https://ziplark.com/privacy/',
  },
  'zh-Hant': {
    name: 'Ziplark',
    subtitle: '全格式解壓縮：RAR、7z、ZIP、tar',
    privacyPolicyUrl: 'https://ziplark.com/privacy/',
  },
  'en-US': {
    name: 'Ziplark',
    subtitle: 'RAR, 7z, ZIP & Tar Archiver',
    privacyPolicyUrl: 'https://ziplark.com/privacy/',
  },
  'ja': {
    name: 'Ziplark',
    subtitle: 'RAR・7z・ZIP・tar 解凍＆圧縮',
    privacyPolicyUrl: 'https://ziplark.com/privacy/',
  },
};

const zhHans = {
  description: `Ziplark 是一款极速、轻量、完全本地运行的全格式归档与解压缩工具——无广告、无内购、不收集任何数据。

• 全格式读取与解压：支持 ZIP（含分卷 .z01/.zip.001、JAR/APK/WHL/CRX/EPUB）、RAR / RAR5（含加密文件名、多分卷 .part01.rar / .r00、自解压 SFX、损坏归档抢救恢复）、7z（含 .7z.001 多分卷）、tar、tar.gz / .bz2 / .xz / .zst / .lz4、ISO 9660 / Joliet 光盘镜像、Debian (.deb) / Unix ar、RPM / CPIO 以及 Microsoft Cabinet (.cab)。
• 免解压快速预览与内部包下钻：双击归档内的任意文件即可直接预览代码/文本、图片或十六进制内容；双击内部嵌套的子压缩包可直接下钻浏览。
• 归档内全文搜索（Grep）：无需解压到磁盘，直接在内存中流式搜索压缩包内所有文本文件的代码或关键字。
• 智能防炸弹解压（Smart Extract）：多根目录压缩包自动归纳进同名子文件夹，单根目录压缩包直接展开不产生双层嵌套。
• 创建、编辑与一步转换：支持创建 AES-256 加密的 ZIP 与 7z，以及各类 tar.* 归档；支持向现有归档追加文件、原地删除条目，或将 RAR/CAB/DEB/RPM/ISO 一键转换为 ZIP、7z 或 tar.zst。
• 自动识别中文/日文/韩文旧编码文件名（GB18030 / Big5 / Shift-JIS / EUC-KR），彻底告别解压乱码。`,
  keywords: '解压,rar,7z,zip,tar,解压缩,压缩包,iso,加密压缩,分卷解压,预览,无广告',
  promotionalText: '极速、纯净的全格式解压缩工具 —— 支持 RAR5、7z、ZIP、tar、ISO 预览、全文搜索与格式转换。',
  supportUrl: 'https://ziplark.com/privacy/',
  marketingUrl: 'https://ziplark.com/privacy/',
  whatsNew: `• 新增归档内全文搜索（Search contents）：无需解压即可检索压缩包内的所有文本文件
• 新增原地移除归档条目（Remove selected）与一键格式转换（Convert…）
• 支持双击压缩包内的嵌套子压缩包直接下钻浏览`,
};

const zhHant = {
  description: `Ziplark 是一款極速、輕量、完全本機執行的全格式封存與解壓縮工具——無廣告、無內購、不收集任何資料。

• 全格式讀取與解壓：支援 ZIP（含分割檔 .z01/.zip.001、JAR/APK/WHL/CRX/EPUB）、RAR / RAR5（含加密檔名、多分割檔 .part01.rar / .r00、自解壓 SFX、損毀封存檔搶救）、7z（含 .7z.001 多分割檔）、tar、tar.gz / .bz2 / .xz / .zst / .lz4、ISO 9660 / Joliet 光碟映像、Debian (.deb) / Unix ar、RPM / CPIO 以及 Microsoft Cabinet (.cab)。
• 免解壓快速預覽與巢狀封存檔瀏覽：雙擊封存檔內的任意檔案即可直接預覽文字、圖片或十六進位內容；雙擊內部子壓縮檔可直接開啟。
• 封存檔內全文搜尋：無需解壓到磁碟，直接在記憶體中搜尋壓縮檔內所有文字檔的關鍵字。
• 智慧防炸彈解壓：自動避免散落滿桌面的檔案或雙層重複資料夾。
• 建立、編輯與一步轉換：支援建立 AES-256 加密的 ZIP 與 7z，以及各類 tar.* 封存檔；支援追加檔案、原地刪除項目，或將 RAR/CAB/DEB/RPM/ISO 一鍵轉換為 ZIP、7z 或 tar.zst。
• 自動識別 Big5 / GB18030 / Shift-JIS / EUC-KR 舊編碼檔名，徹底告別亂碼。`,
  keywords: '解壓縮,rar,7z,zip,tar,壓縮檔,解壓,iso,加密壓縮,分割檔,預覽,無廣告',
  promotionalText: '極速、純淨的全格式解壓縮工具 —— 支援 RAR5、7z、ZIP、tar、ISO 預覽、全文搜尋與格式轉換。',
  supportUrl: 'https://ziplark.com/privacy/',
  marketingUrl: 'https://ziplark.com/privacy/',
  whatsNew: `• 新增封存檔內全文搜尋（Search contents）
• 新增原地移除項目與一鍵格式轉換（Convert…）
• 支援雙擊內部巢狀子壓縮檔直接開啟瀏覽`,
};

const enUS = {
  description: `Ziplark is a fast, lightweight, 100% local archive manager for macOS — no ads, no subscriptions, and zero telemetry.

• Extract & inspect every format: ZIP (including split .z01/.zip.001, JAR, APK, WHL, CRX, EPUB), RAR & RAR5 (including encrypted headers, multi-volume .part01.rar / .r00 sets, self-extracting .exe archives, and damaged-archive file salvage), 7z (including .7z.001 multi-volume), tar, tar.gz / .bz2 / .xz / .zst / .lz4, ISO 9660 / Joliet disc images, Debian (.deb) & Unix ar, RPM & CPIO, and Microsoft Cabinet (.cab).
• Quick Preview & Nested Archive Drill-Down: Double-click any file inside an archive to preview UTF-8 code/text, images, or hex dumps in memory without extracting to disk. Double-click a nested archive to browse inside it directly.
• In-Archive Full-Text Search (Grep): Search across text files inside any archive in memory, automatically skipping binary files.
• Smart Anti-Tarbomb Extraction: Multi-root archives are neatly wrapped into a clean folder, while single-root archives extract directly without double-nested folders.
• Create, Modify & Convert: Create AES-256 encrypted ZIP and 7z archives or compressed tarballs; add or remove entries in place; convert RAR, CAB, DEB, RPM, or ISO archives directly into ZIP, 7z, or tar.zst.
• Automatic Legacy Filename Decoding: Detects Shift-JIS, GB18030, Big5, EUC-KR, and Windows code pages automatically so non-UTF-8 filenames never turn into mojibake.`,
  keywords: 'rar,7z,zip,unarchiver,extract,archive,tar,iso,unrar,decompressor,aes256,grep',
  promotionalText: 'Fast, clean archiver for ZIP, RAR5, 7z, tar, ISO, DEB, RPM & CAB — with instant preview, in-archive search, and format conversion.',
  supportUrl: 'https://ziplark.com/privacy/',
  marketingUrl: 'https://ziplark.com/privacy/',
  whatsNew: `• In-archive full-text search ("Search contents"): grep across text files inside archives without extracting to disk
• In-place entry deletion ("Remove selected") for ZIP, 7z, and tar.* archives
• One-step archive format conversion ("Convert…") and nested archive drill-down`,
};

const ja = {
  description: `Ziplark は高速・軽量・完全ローカル動作のアーカイブ（解凍・圧縮）ツールです。広告なし、課金なし、データ収集も一切ありません。

• 幅広い形式の展開・閲覧：ZIP（分割 .z01/.zip.001、JAR/APK/WHL/CRX/EPUB 含む）、RAR / RAR5（暗号化ヘッダー、分割 .part01.rar / .r00、自己解凍 SFX、破損アーカイブの救出展開）、7z（.7z.001 分割含む）、tar（.gz / .bz2 / .xz / .zst / .lz4）、ISO 9660 / Joliet、Debian (.deb) / ar、RPM / CPIO、Microsoft Cabinet (.cab) に対応。
• クイックプレビュー＆ネストされたアーカイブの直接展開：ファイルをダブルクリックしてテキスト・画像・16進ダンプをメモリ上で即座にプレビュー。アーカイブ内の子アーカイブもそのまま開けます。
• アーカイブ内全文検索（Grep）：ディスクに展開せず、アーカイブ内のテキストファイルを一括検索。
• 文字化け防止：Shift-JIS (CP932)・GB18030・Big5・EUC-KR などの古い文字コードを自動検出し、日本語ファイル名の文字化けを防ぎます。
• 作成・編集・形式変換：AES-256 暗号化対応の ZIP / 7z や tar.* の作成、ファイルの追加・削除、RAR 等から ZIP / 7z / tar.zst への一括変換に対応。`,
  keywords: '解凍,rar,7z,zip,tar,アーカイブ,圧縮,文字化け,iso,展開,プレビュー',
  promotionalText: 'RAR5・7z・ZIP・tar・ISO 対応の高速アーカイバー。プレビュー、全文検索、文字化け自動修復、形式変換を搭載。',
  supportUrl: 'https://ziplark.com/privacy/',
  marketingUrl: 'https://ziplark.com/privacy/',
  whatsNew: `• アーカイブ内の全文検索（Search contents）を追加
• エントリの直接削除とアーカイブ形式の一括変換（Convert…）を追加
• ネストされた子アーカイブのダブルクリック展開に対応`,
};

export const versionLocalizations = { 'zh-Hans': zhHans, 'zh-Hant': zhHant, 'en-US': enUS, 'ja': ja };

export const appFields = {
  primaryCategory: 'UTILITIES',
  contentRightsDeclaration: 'DOES_NOT_USE_THIRD_PARTY_CONTENT',
};

export const ageRating = {
  alcoholTobaccoOrDrugUseOrReferences: 'NONE',
  contests: 'NONE',
  gambling: false,
  gamblingSimulated: 'NONE',
  gunsOrOtherWeapons: 'NONE',
  healthOrWellnessTopics: false,
  horrorOrFearThemes: 'NONE',
  lootBox: false,
  matureOrSuggestiveThemes: 'NONE',
  medicalOrTreatmentInformation: 'NONE',
  profanityOrCrudeHumor: 'NONE',
  sexualContentGraphicAndNudity: 'NONE',
  sexualContentOrNudity: 'NONE',
  unrestrictedWebAccess: false,
  userGeneratedContent: false,
  violenceCartoonOrFantasy: 'NONE',
  violenceRealistic: 'NONE',
  violenceRealisticProlongedGraphicOrSadistic: 'NONE',
  advertising: false,
  ageAssurance: false,
  messagingAndChat: false,
  parentalControls: false,
  socialMedia: false,
};

const dir = new URL('../../appstore-assets/', import.meta.url).pathname;
export const screenshots = {
  MAC_OS: {
    dir,
    sets: [{ displayType: 'APP_DESKTOP', files: ['mac-1.png', 'mac-2.png', 'mac-3.png'] }],
  },
};

export const reviewDetail = {
  contactFirstName: 'Xiangdong',
  contactLastName: 'Li',
  contactPhone: '+8618588279123',
  contactEmail: 'lixd220@gmail.com',
  demoAccountRequired: false,
  notes: `Ziplark is a local archive utility (ZIP, RAR, 7z, tar, ISO).

1. Click "Choose archive…" (or drag any .zip / .7z / .tar.gz / .rar file onto the window) to inspect entries.
2. Double-click any file row to open Quick Preview in memory, or click "Extract all…" / "Extract here" to extract.
3. Switch to the "Create" tab at the top to create a new .zip, .7z, or .tar.* archive from local files.

All processing runs 100% locally on the Mac with zero network or account requirements.`,
};

export const platforms = {
  MAC_OS: {
    versionString: '0.5.0',
    buildVersion: '1',
    copyright: '© 2026 doaipm',
  },
};
