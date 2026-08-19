# hashai 設計書

## 1. 概要

hashai は、LinuxおよびmacOS上のBash、Zsh、Fishの対話プロンプトに入力された自然言語を、Codex CLIを使って現在の環境に適したシェルコマンドへ変換するツールである。

想定する操作は次のとおり。

```text
# 現在のディレクトリ以下で 1GB 以上のファイルをリストアップして
```

変換操作を行うと、現在の入力行が次のようなコマンドに置き換わる。

```bash
find . -type f -size +1073741824c -print
```

生成されたコマンドは自動実行しない。利用者が内容を確認・編集し、通常の Enter で実行する。

## 2. 目的

- 覚えにくいオプションやパイプラインを自然言語から組み立てられるようにする。
- ターミナルから離れずにコマンドを生成できるようにする。
- Bash、Zsh、Fish で一貫した操作感を提供する。
- LinuxとmacOSのコマンド差を考慮したコマンドを生成する。
- Codex CLIの既存認証を再利用する。
- AI が生成したコマンドを人間が確認してから実行する境界を維持する。

## 3. 非目標

- 自然言語の入力直後にコマンドを無確認で実行すること。
- Bash、Zsh、Fish の構文差を無視して、すべてを Bash コマンドとして生成すること。
- Codex CLI に代わる独自のモデルクライアントや認証機構を実装すること。
- 完全なシェル、ターミナルエミュレーター、常駐型エージェントを作ること。
- Windows、FreeBSD・OpenBSD・NetBSD、BusyBox環境を初期リリースで正式サポートすること。
- 過去の依頼を記憶する会話型・multi-turnインターフェースを提供すること。
- 単純な禁止語リストだけで、生成コマンドの安全性を保証すること。

## 4. 基本UX

### 4.1 推奨操作

自然言語をコメント形式で入力し、`Ctrl+G` で変換する。

```text
# git のコミットとプッシュを実行して
```

変換後:

```bash
git add -A && git commit && git push
```

`Enter` は通常どおりシェルにコマンドを実行させる。変換と実行に異なるキーを割り当てることで、意図しない実行を防ぐ。

### 4.2 代替操作

キーバインドが利用できない環境向けに、生成コマンドを標準出力へ表示する明示的なコマンドも提供する。外部プロセスから呼び出し元シェルの入力バッファは変更できないため、入力バッファへの挿入はシェル統合を経由する場合に限る。

```bash
hashai generate '1GB 以上のファイルを探して'
```

短縮エイリアスを提供する場合は、以下を候補とする。

```bash
?? '1GB 以上のファイルを探して'
```

`??` は各シェルに適したエイリアスまたは略記として提供し、`hashai generate` と同様に生成コマンドを標準出力へ表示する。入力バッファへの挿入は `Ctrl+G` のシェル統合だけが担当する。

### 4.3 `#` と通常コメントの衝突

`#` は本来シェルのコメントである。スクリプトや複数行テキストを貼り付けた際の誤検出を避けるため、次を仕様に含める。

- `Ctrl+G` を押した場合だけ変換する。
- 入力行が設定済みトリガーで始まらなければ変換しない。
- トリガーを `# `、`,,` などへ変更できるようにする。
- コメント形式のフックを無効化し、明示的コマンドだけを使えるようにする。
- 変換対象からトリガー部分を除去してCodexへ渡す。

## 5. アーキテクチャ

```text
Bash: READLINE_LINE / bind -x ----+
Zsh:  BUFFER / ZLE widget --------+--> hashai core --> codex exec
Fish: commandline / bind ---------+         |
                                            +--> command / metadata
```

実装を次の2層に分離する。

### 5.1 Core

シェルに依存しない共通実行ファイル。責務は以下。

1. 自然言語と実行環境情報を受け取る。
2. Codex用プロンプトを構築する。
3. `codex exec` を読み取り専用で起動する。
4. 構造化された結果を検証する。
5. コマンドと危険度を呼び出し元へ返す。
6. タイムアウト、キャンセル、エラーを統一的に処理する。

### 5.2 Shell integration

シェル固有の薄い統合コード。責務は以下。

1. 現在の入力バッファとカーソル位置を取得する。
2. 入力が変換対象か判定する。
3. Coreを呼び出す。
4. 生成されたコマンドを入力バッファへ戻す。
5. カーソルを適切な位置へ移動する。
6. 失敗時に元の入力を保持する。

## 6. シェル別統合

### 6.1 Bash

- `bind -x` で `Ctrl+G` に関数を割り当てる。
- `READLINE_LINE` から入力を取得する。
- `READLINE_POINT` を更新してカーソルを末尾へ移動する。
- Coreが失敗した場合は `READLINE_LINE` を変更しない。

### 6.2 Zsh

- ZLE widgetとして実装する。
- `BUFFER` から入力を取得し、結果を再設定する。
- `CURSOR=${#BUFFER}` でカーソルを末尾へ移動する。

### 6.3 Fish

- `bind` で `Ctrl+G` にFish関数を割り当てる。
- `commandline` で入力を取得・置換する。
- FishはBash/Zshと構文が異なるため、プロンプトへ `fish` を明示する。
- コマンド置換時の改行分割とクォートに注意する。

### 6.4 配布方法

Coreから統合スクリプトを生成する方式を採用する。

```bash
hashai integration generate bash
hashai integration generate zsh
hashai integration generate fish
```

生成済みファイルを各シェルの設定からsourceする。シェル起動のたびにCoreを起動して統合コードを生成する方式は、起動時間と障害範囲の面からデフォルトにしない。

artifact はユーザーのhashaiデータディレクトリ配下の `integrations/hashai.<shell>` にだけ保存する。`generate` は指定shellのartifactを作成し、`update` は導入済みartifactだけを更新し、`list` は副作用なく導入済みartifactのshell・version・状態・pathをタブ区切りで表示する。artifactにはversion markerを含める。更新は同じ管理ディレクトリ内のatomic renameで行い、既存の通常ファイルを `hashai.<shell>.bak` に保存する。管理対象のディレクトリ、artifact、backupがsymlinkまたは通常ファイル以外の場合は拒否し、任意パスへの出力は提供しない。

### 6.5 対応環境

- Linux: glibcを使用するx86_64およびaarch64環境。
- macOS: macOS 13以降のIntel MacおよびApple Silicon Mac。
- Bash: 4.0以降。macOS標準の古いBashではなく、Homebrewなどで導入した対応バージョンを使用する。
- Zsh: 5.8以降。
- Fish: 3.6以降。
- Codex CLI: 固定の最低バージョン番号ではなく、hashaiが必要とするコマンドとフラグをすべて備えたバージョンを要求する。

## 7. Codex CLIとの連携

### 7.1 基本呼び出し

概念上の呼び出しは以下。

```bash
codex exec - \
  --ephemeral \
  --ignore-user-config \
  --ignore-rules \
  --model gpt-5.6-luna \
  --config 'model_reasoning_effort="low"' \
  --config 'project_doc_max_bytes=0' \
  --config 'project_doc_fallback_filenames=[]' \
  --sandbox read-only \
  --disable shell_tool \
  --disable browser_use \
  --disable computer_use \
  --disable apps \
  --skip-git-repo-check \
  --output-schema /path/to/schema.json
```

- `--ephemeral`: コマンド変換ごとのセッションを保存しない。
- `--ignore-user-config`: Codexのユーザー設定、MCP、hooksなどを継承せず、認証だけを再利用する。
- `--ignore-rules`: プロジェクトのexecpolicyルールを読み込まない。
- `--model`: hashaiの設定で選択されたモデルをCodexへ明示する。
- `--config model_reasoning_effort=...`: hashaiの設定で選択された推論強度をCodexへ明示する。
- `--config project_doc_max_bytes=0` と `project_doc_fallback_filenames=[]`: `AGENTS.md` と代替プロジェクト文書をプロンプトへ混入させない。
- `--sandbox read-only`: 生成処理中の書き込みを許可しない。
- `--disable shell_tool`、`browser_use`、`computer_use`、`apps`: コマンド生成に不要なagent toolsを無効化する。
- `--skip-git-repo-check`: Gitリポジトリ外でも利用可能にする。
- `--output-schema`: 説明文やMarkdownが混ざらない構造化出力を要求する。
- 自然言語は引数展開ではなく標準入力で渡し、クォート問題を減らす。

実行時の作業ディレクトリは現在のディレクトリと一致させる。ただし、コマンド変換だけならCodexがディレクトリ内容を調査する必要はない。コンテキスト調査を許可するかは設定として分離する。

Codex CLIはユーザー設定やプロジェクト文書を無効化しても、Codex固有の組み込みagent instructionsを送信する。このため、直接APIを呼び出す単純な変換器より入力トークン、クォータ消費、レイテンシが大きくなる可能性がある。hashaiはCodex CLIの既存認証を再利用する利点と引き換えに、この負担を受け入れる。

### 7.2 出力スキーマ

初期実装では次の形式を使用する。

```json
{
  "type": "object",
  "properties": {
    "command": {
      "type": "string",
      "minLength": 1
    },
    "risk": {
      "type": "string",
      "enum": ["safe", "review", "dangerous"]
    }
  },
  "required": ["command", "risk"],
  "additionalProperties": false
}
```

通常の入力バッファまたは標準出力には `command` だけを出力する。`risk` が `review` または `dangerous` の場合は、診断用の標準エラーへ一方向の警告を表示する。確認選択や説明などの専用対話UIは設けない。

JSON SchemaはRustバイナリへ埋め込み、Codex実行時に排他的作成を使って権限 `0600` の一時ファイルへ書き出す。成功、失敗、キャンセルのいずれでもRAIIにより削除する。既存ファイルや予測可能な固定パスを再利用しない。

### 7.3 プロンプト方針

プロンプトには最低限、次を含める。

- コマンドを実行せず、生成だけを行う。
- 対象OS、ディストリビューション、シェルを指定する。
- 現在の作業ディレクトリを指定する。
- 不要な `sudo` を使用しない。
- 破壊的操作や不可逆操作は `dangerous` と判定し、利用者の意図を満たせる場合は回復可能な代替手段を優先する。
- 曖昧な値を捏造しない。
- コミットメッセージなどが不明なら、対話的なコマンドを選ぶ。
- 複数の操作は順序依存がある場合だけ `&&` で接続する。
- Bash向けの構文をFishへ出力しない。
- 構造化スキーマに従う。

シェル別のfew-shot例を用意し、OSやシェルに固有の慣用的なコマンドを誘導する。ただし例が生成結果を過度に固定しないよう、小さく保つ。

## 8. コンテキスト

### 8.1 デフォルトで送信する候補

- 対象シェルとバージョン
- OS、CPUアーキテクチャ、LinuxディストリビューションまたはmacOSバージョン
- 主要コマンドがGNU版、BSD版のどちらかを判定するためのバージョン情報
- 現在の作業ディレクトリの論理パス
- Gitリポジトリ内かどうか
- 設定されたコマンド生成方針

### 8.2 オプトインにする候補

- 近隣ファイル名
- Git status
- プロジェクト種別
- package managerや代表的な設定ファイル

ファイル内容、Git差分、環境変数、シークレットはデフォルトで送信しない。コンテキストを追加する場合は、何を送るか利用者が確認できる診断機能を用意する。

## 9. Single-turn

hashaiはsingle-turnのみを提供し、会話状態を保持しない。

本ツールの目的は、現在の入力行に書かれた自然言語を、その時点の環境に適したシェルコマンドへ変換することである。過去の依頼を参照するmulti-turn機能はこの目的に必要なく、次の問題を生むため対象外とする。

- 同じ入力でも会話履歴によって結果が変わり、予測可能性が下がる。
- 誤った生成結果が後続の変換へ影響する。
- 履歴の表示、編集、削除、保存期間など追加の状態管理が必要になる。
- 意図せず過去のコマンドや作業情報をモデルへ再送信する可能性がある。
- コマンド生成とエージェント型タスク実行の境界が曖昧になる。

「それを元に戻して」のように過去の依頼を前提とする入力は解決しない。必要な情報を含む独立した依頼として入力し直してもらう。

## 10. 安全性

### 10.1 基本原則

安全境界は「AIに実行させない」「入力バッファへ戻し、人間が確認する」である。生成結果を `eval` や同等の方法で即時実行してはならない。

禁止する実装例:

```bash
eval "$(hashai generate '不要ファイルを消して')"
```

### 10.2 警告対象

以下を含む場合は、少なくとも `review` または `dangerous` として扱う。

- `rm`、`rmdir`、ファイルの上書き
- `sudo`、`su`
- `dd`、`mkfs`、パーティション操作
- `chmod -R`、`chown -R`
- `git reset --hard`、`git clean`
- `git push --force`、`git push -f`
- `curl ... | sh` や同等のリモートコード実行
- プロセスの一括終了
- データベースの削除・切り詰め
- 複数行スクリプト、heredoc、複雑なコマンド置換

モデルの `risk` とローカル解析の判定を比較し、より高い危険度を最終判定として採用する。ローカル解析は危険度を引き上げることだけができ、モデルの判定を引き下げない。初期実装では、危険なコマンド、オプション、リダイレクト、パイプ、複数行構文を保守的に検出する。

文字列やトークンに基づく検査は補助的な警告に留める。難読化、エイリアス、スクリプト経由などを完全には検出できないためであり、ローカル解析も安全性を保証する境界とはしない。

## 11. エラー処理

- Codexが見つからない場合は、インストール確認方法を表示する。
- 未認証の場合は、Codexのログイン方法を案内する。
- 設定されたモデルまたは推論強度を利用できない場合は、自動的に別の値へフォールバックせず、設定変更を案内する。
- タイムアウト時は元の入力バッファを保持する。
- `Ctrl+C` でCodex子プロセスを終了できるようにする。
- 空または不正な構造化出力を入力バッファへ挿入しない。
- Coreの診断メッセージと生成コマンドの出力チャネルを分離する。
- 非TTY環境ではインライン操作を無効化し、非対話モードへ切り替える。
- 終了コードを定義し、シェル統合が失敗理由を判定できるようにする。

### 11.1 終了コード

| 終了コード | 意味 |
|---:|---|
| `0` | コマンドの生成に成功した。 |
| `1` | 分類できない一般的な実行エラーが発生した。 |
| `2` | CLI引数またはhashaiの設定が不正である。 |
| `3` | Codex CLIが見つからない。 |
| `4` | Codex CLIが認証されていない。 |
| `5` | 設定されたモデルまたは推論強度を利用できない。 |
| `6` | Codex CLIの実行がタイムアウトした。 |
| `7` | 利用者が処理をキャンセルした。 |
| `8` | Codexの出力が不正、空、またはJSON Schemaに違反している。 |
| `9` | OS、シェル、またはそのバージョンがサポート対象外である。 |

シェル統合は終了コードが `0` の場合だけ入力バッファを書き換える。その他の場合は元の入力を保持し、診断を標準エラーへ表示する。

## 12. 設定

設定候補:

```toml
trigger = "# "
keybinding = "ctrl-g"
shell = "auto"
timeout_seconds = 30
context = "minimal"
[codex]
model = "gpt-5.6-luna"
reasoning_effort = "low"

[prompt]
extra_instructions = "Prefer rg over grep when available."
```

優先順位は、CLI引数、環境変数、ユーザー設定、組み込みデフォルトの順とする。初期リリースでは未信頼リポジトリから挙動を変更されないよう、プロジェクト設定を読み込まない。`prompt.extra_instructions` はユーザー設定でのみ指定できる。`review` と `dangerous` の警告は無効化できない。APIキーをhashai自身の設定ファイルへ保存せず、Codex CLIの認証を利用する。

## 13. CLI案

```text
hashai generate <natural-language>
hashai integration generate <bash|zsh|fish>
hashai integration update
hashai integration list
hashai config show
hashai doctor
```

スクリプト利用向けに次を検討する。

```text
--format command
--format json
--shell bash|zsh|fish
--context none|minimal|project
--timeout <seconds>
```

`hashai doctor` は少なくとも以下を確認する。

- Codex CLIの存在とバージョン
- `exec`、`--ephemeral`、`--ignore-user-config`、`--ignore-rules`、`--output-schema`、`--sandbox`、必要な `--disable` 対象とプロジェクト文書無効化設定の有無
- Codexの認証状態
- 設定されたモデルと推論強度が利用可能か
- 対象シェルとバージョン
- キーバインド競合
- 統合ファイルのバージョン不一致
- JSON処理に必要な依存関係
- Gitリポジトリ外での動作確認

## 14. 類似ツールから採用する要素

### Deltik/shell-ai

- Bash、Zsh、Fish向け統合ファイルの生成。
- `??` エイリアスと `Ctrl+G` の併用。
- 非対話モードとJSON出力。
- シェル起動時の負荷を避ける静的統合ファイル。
- Codexをread-only sandboxで起動し、shell、browser、computer、appsのagent toolsを無効化する方式。

### Scout

- Codexの `--ignore-user-config` と `--ignore-rules` を使用し、対話エージェント向けの設定、MCP、hooks、プロジェクトルールを継承しない方式。
- `--ephemeral`、read-only sandbox、標準入力、出力用一時ファイルを組み合わせる方式。

### ShellGPT

- OSとシェルを考慮した生成。
- 生成コマンドを入力バッファへ戻して編集可能にする方式。

### Warp AI

- `#` を自然言語モードとして認識させる操作モデル。
- 生成中・通常コマンド・自然言語入力を視覚的に区別する考え方。

### zsh-ai

- `# ...` をコマンドへ変換し、二度目のEnterで実行する方式。
- トリガーの変更とコメントフックの無効化。
- OS、Git、プロジェクト情報を限定的に利用する方式。

### llm-cmd

- 小さな「コマンドだけを返す」プロンプト。
- 編集可能な状態で表示し、`Ctrl+C` で中止する方式。
- 危険性を利用者へ明示する姿勢。

### Microsoft/Codex-CLI

- コメント形式の入力と `Ctrl+G` の組み合わせ。
- シェル別few-shot例。
- single-turnをデフォルトにする考え方。

## 15. 実装フェーズ

### Phase 1: 最小実装

- 共通Core。
- `codex exec` とJSON Schemaの連携。
- `hashai generate`。
- Bash、Zsh、Fishを明示指定した生成。
- タイムアウトとキャンセル。
- 単体テスト可能なプロンプト構築と出力検証。

### Phase 2: シェル統合

- BashのReadline統合。
- ZshのZLE統合。
- Fishの`commandline`統合。
- `Ctrl+G` で入力バッファを置換。
- 統合スクリプト生成、更新、一覧表示。
- 元入力を保持するエラー処理。

### Phase 3: 安全性とUX

- 危険度表示。
- キーバインドとトリガーの設定。
- `hashai doctor`。

## 16. テスト方針

- Coreの単体テストではCodexプロセスを差し替え、正常、空出力、不正JSON、タイムアウト、キャンセルを検証する。
- Bash、Zsh、Fishごとに入力バッファ取得・置換を統合テストする。
- Linux/macOSとBash/Zsh/Fishを組み合わせた6環境のテストマトリクスを用意する。
- 空白、引用符、改行、日本語、絵文字を含む入力を検証する。
- 危険なコマンドが自動実行されないことをテストする。
- Gitリポジトリ内外の双方で動作を検証する。
- Linuxディストリビューション差とmacOSバージョン差をテストケース化する。
- LinuxではGNU coreutils・findutils、macOSでは標準BSD系コマンドを基準環境として統合テストする。
- `find`、`stat`、`date`、`sed`、`xargs` など、GNU版とBSD版で差が大きいコマンドを両環境で評価する。
- 生成品質は固定スナップショットだけで判定せず、スキーマ準拠と要求された性質を評価する。

代表的な評価入力:

```text
# 現在のディレクトリ以下で 1GB 以上のファイルをリストアップして
# 直近7日で変更されたJSONファイルをサイズ順に表示して
# git のコミットとプッシュを実行して
# 3000番ポートを使用しているプロセスを調べて
# このディレクトリを削除して
```

## 17. 決定事項

- Coreの実装言語はRustとする。単一バイナリ、起動速度、型による出力検証、Bash・Zsh・Fish向け統合ファイルの生成に適しているためである。
- Codexのモデルと推論強度はhashaiの設定で変更可能にする。
- デフォルトモデルは `gpt-5.6-luna` とする。短いコマンド生成を高頻度かつ低コストで実行する用途に合うためである。
- デフォルト推論強度は `low` とする。Codex CLIで選択できる最小の `minimal` よりも、クォート、パイプライン、OS・シェル差、危険度判定を考慮する余地を持たせつつ、対話プロンプトで重要な低遅延性を優先する。
- `minimal` は低遅延を最優先する利用者向けの設定値として許可する。ただしデフォルトへ変更する場合は、代表的な入力について `low` と生成品質、危険度判定、レイテンシを比較して判断する。
- モデルの危険度判定をローカル解析で補正し、両者の高い方を最終的な危険度とする。ローカル解析は警告のための補助であり、安全性を保証するものではない。
- 初期サポート対象はLinuxとmacOSとする。LinuxではGNU coreutils・findutils、macOSでは標準BSD系コマンドを基準とする。
- OS、バージョン、利用可能な主要コマンドの情報をCodexへ渡し、GNU/BSD差を考慮したコマンドを生成させる。hashai自身にはGNUコマンドとBSDコマンドを相互変換する層を実装しない。
- Windows、その他のBSD、BusyBoxは初期サポート対象外とする。
- Codexのユーザー設定、プロジェクトルール、`AGENTS.md`、代替プロジェクト文書は継承せず、既存認証だけを再利用する。shell、browser、computer、appsのagent toolsを明示的に無効化する。
- プロジェクト設定は読み込まず、CLI引数、環境変数、ユーザー設定、組み込みデフォルトだけを使用する。危険度警告は無効化できない。
- 設定されたモデルまたは推論強度が利用できない場合は自動フォールバックせず、元の入力バッファを保持して設定変更を案内する。
- ライセンスはMIT Licenseとする。
- 開発中は `cargo install --path .` でローカルインストールする。初期配布はGitHub Releasesのビルド済みバイナリを使用し、安定後にcrates.ioとHomebrew tapへの公開を検討する。
- 初期のGitHub Releasesでは `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`x86_64-apple-darwin`、`aarch64-apple-darwin` の4ターゲットを配布する。
- CLIの終了コードはセクション11.1の契約に従い、シェル統合が失敗時に入力バッファを保持できるようにする。

初期判断としては、Rust製の単一バイナリ、single-turn、`Ctrl+G`、Codex CLIの既存認証を優先する。生成結果は自動実行せず、シェル統合では入力バッファへ挿入し、明示的CLIでは標準出力へ表示する。

## 18. 参考資料

- [OpenAI: Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- [OpenAI: Codex developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
- [Deltik/shell-ai](https://github.com/Deltik/shell-ai)
- [tomnagengast/scout](https://github.com/tomnagengast/scout)
- [TheR1D/shell_gpt](https://github.com/TheR1D/shell_gpt)
- [Warp AI](https://www.warp.dev/warp-ai)
- [Warp command entry](https://docs.warp.dev/terminal/entry)
- [matheusml/zsh-ai](https://github.com/matheusml/zsh-ai)
- [simonw/llm-cmd](https://github.com/simonw/llm-cmd)
- [microsoft/Codex-CLI](https://github.com/microsoft/Codex-CLI)
- [sigoden/aichat](https://github.com/sigoden/aichat)
