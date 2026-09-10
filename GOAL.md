
I am using Pixi and https://prefix.dev/github-releases channel.
This channel has `rust-analyzer` https://prefix.dev/channels/github-releases/packages/rust-analyzer
but the versions are out-of-date:
```
$ pixi search rust-analyzer --channel https://prefix.dev/github-releases
Using channels: https://prefix.dev/github-releases/
rust-analyzer-2026-he8cfe8b_0 (+ 1 build)
-----------------------------------------

Name                rust-analyzer
Version             2026
Build               he8cfe8b_0
Size                12.59 MiB
License             Apache-2.0
Timestamp           2026-03-13 10:36:08 UTC
Subdir              linux-aarch64
File Name           rust-analyzer-2026-he8cfe8b_0.conda
URL                 https://prefix.dev/github-releases/linux-aarch64/rust-analyzer-2026-he8cfe8b_0.conda
MD5                 0fdf9cedb25489771707af575f5ca0d5
SHA256              aa0af784bfdfc2eb941490c2b26e94fc2a04c0cbf5dc626ab1a1b21a97ea2755

Dependencies:

Run exports:

Other Versions (6):
Version  Build
2025     he8cfe8b_0 (+ 1 build)
2024     he8cfe8b_0 (+ 1 build)
2023     he8cfe8b_0 (+ 1 build)
2022     he8cfe8b_0 (+ 1 build)
2021     he8cfe8b_0 (+ 1 build)
... and 1 more
```

As you can see in https://github.com/rust-lang/rust-analyzer/releases, the latest rust-analyzer releases is `2026-09-07`.

In `hunger/octoconda` repo (that you are in currently),
I run:
```
pixi run build-one rust-lang/rust-analyzer
```
Output:
```
✨ Pixi task (build-one): bash scripts/build_one.sh rust-lang/rust-analyzer
==> Generating recipes for rust-lang/rust-analyzer into test-output/
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.62s
     Running `target/debug/octoconda --filter '^rust-lang/rust-analyzer$' --work-dir test-output --keep-temporary-data`
Note: Duplicate package name "aws-vault": produced by both "99designs/aws-vault" and "ByteNess/aws-vault"(at least one is deprecated)
Note: Duplicate package name "jiq": produced by both "bellicose100xp/jiq" and "fiatjaf/jiq"(at least one is deprecated)
Note: Duplicate package name "kubecolor": produced by both "hidetatz/kubecolor" and "kubecolor/kubecolor"(at least one is deprecated)
Note: Duplicate package name "golines": produced by both "golangci/golines" and "segmentio/golines"(at least one is deprecated)
Processing 1 packages (1 need updates, 0 fully-imported spot-checks)
Processed 1/1 repositories: rust-lang/rust-analyzer

Unknown packages in conda (106):
  abtop
  agent-of-empires
  alexhallam__tv
  amplify-cli
  apfel
  app-store-connect-cli
  aqua
  archisteamfarm
  archon
  asdf
  autorestic
  baguette
  boilr
  circumflex
  claurst
  cli-printing-press
  clipboard
  cml
  cmux
  code2prompt
  codemachine-cli
  codewhale
  convoy
  deepseek-reasonix
  dekit
  destructive_command_guard
  dsq
  elio
  evolver
  fallow
  fselect
  fuxi
  gdscript-formatter
  gitlogue
  gitmoji-cli
  gitoxide
  gmailctl
  go-musicfox
  gobackup
  googleworkspace__cli
  gowall
  gpg-tui
  gptme
  grepai
  helix-db
  heretek
  hexapoda
  hishtory
  html-to-markdown
  httplab
  hunk
  hygen
  inngest
  inshellisense
  ipatool
  jcode
  jscpd
  kilocode
  kondo
  landrun
  ludusavi
  lumen
  mcporter
  md2wechat-skill
  mimo-code
  mob
  mole
  nali
  nba-go
  nexe
  nexrender
  nixpacks
  octosql
  officecli
  oh-my-pi
  opensrc
  ots
  ouroboros
  pdfcpu
  psmux
  qawolf__cli
  qrcp
  rerun
  restate
  resterm
  revylai__greenlight
  rhubarb-lip-sync
  rmux
  rtk
  semtools
  skillshare
  so-novel
  squad
  summarize
  surge
  systemd-lsp
  tirith
  tldx
  typeshare
  viu
  wacli
  wasm-bindgen__wasm-pack
  weathr
  webclaw
  yek
  yt-dlp


No recipes generated for rust-lang/rust-analyzer
```

Why it does not work?
Note the latest `rust-analyzer` releases has these 20 assets:
```
rust-analyzer-aarch64-apple-darwin.gz sha256:16e9b2af9db7c0ce015ffe88f85db27669b05c84887a59c737d697d2c5f8d349 13.2 MB 3 days ago
rust-analyzer-aarch64-pc-windows-msvc.zip sha256:2cbb41535af5d6283b56866393d17b2844a6c37dbdd43e9432ffbe40ddd9f29e 14.9 MB 3 days ago
rust-analyzer-aarch64-unknown-linux-gnu.gz sha256:484de96ea9e5daf9e6361b6fa91be9521a7e6c652111af54a7ead80f900b1e55 13.7 MB 3 days ago
rust-analyzer-alpine-x64.vsix sha256:555e32132c1718d0666e7b68d25faa7eb107fba888c0b9158329c0176c5b7a49 15.2 MB 3 days ago
rust-analyzer-arm-unknown-linux-gnueabihf.gz sha256:2dea56aad76229d3f71a61d19d9992e20f4500f2a3fb98cec1442a30c8c5ab46 14.5 MB 3 days ago
rust-analyzer-darwin-arm64.vsix sha256:3c4899966ad94acb3ad787a66cbb501f79ee60edac21b17bec2693fe50994b78 14.6 MB 3 days ago
rust-analyzer-darwin-x64.vsix sha256:bdaa625fdb6162912aa4f9967203728b9b5c8a38adf38d5b58c325ba3149d0a6 15.1 MB 3 days ago
rust-analyzer-i686-pc-windows-msvc.zip sha256:115e026074c706fca97481386629805bb3d051ae0bc9695765a34c282f7110c4 14.6 MB 3 days ago
rust-analyzer-linux-arm64.vsix sha256:f33d35db89c12bf5cb09f3c2280f3ba88e7c34727f93fefb6afec6ffbb766384 15.1 MB 3 days ago
rust-analyzer-linux-armhf.vsix sha256:a190f3958baa91a0388d9ebfa4e2eb39c27215665272eef97018f07687299d89 15.8 MB 3 days ago
rust-analyzer-linux-x64.vsix sha256:0b8ce8b4baddcaecd58980d4778883cdfca273597f7d4c9680338bbaed6ab2f0 15.4 MB 3 days ago
rust-analyzer-no-server.vsix sha256:6fa886c872487f669ac54772c7cc4395d651a4a5d374c8ce9ca203a18c5bd23a 900 KB 3 days ago
rust-analyzer-win32-arm64.vsix sha256:2dc8e5b177ea3c40ceb9bac801bb6596c7f12252644de77d4bf7fb710d08119d 16.5 MB 3 days ago
rust-analyzer-win32-x64.vsix sha256:85595a61ee36ea76896aa1c0c591928286731714e2307e57508e21b74d593c79 18.1 MB 3 days ago
rust-analyzer-x86_64-apple-darwin.gz sha256:41161c05bd7e2396a5cea86a691d37919ec0af331a06bbdc0f323979034f9dad 13.9 MB 3 days ago
rust-analyzer-x86_64-pc-windows-msvc.zip sha256:cd3dddd580edac199c5e84c6cceb3addea32769c25cb6b597024a200b3359b05 16.7 MB 3 days ago
rust-analyzer-x86_64-unknown-linux-gnu.gz sha256:a3500183aa08bf740c0da6e030ad262d4cfa1c19e7ce195ab5f772bdf9ddfb12 14.2 MB 3 days ago
rust-analyzer-x86_64-unknown-linux-musl.gz sha256:d05af6fdc2eab4e2b348f53029b6557369c910ba6cb2006d85cc1fb6ea400c35 14.3 MB 3 days ago
Source code (zip) 4 days ago
Source code (tar.gz) 4 days ago
```
