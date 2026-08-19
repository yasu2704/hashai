# hashai 設計書

## 1. 概要

hashai は、Bash、Zsh、Fish の対話プロンプトに入力された自然言語を、Codex CLI を使って現在の環境に適したシェルコマンドへ変換するツールである。

想定する操作は次のとおり。

```text
# 現在のディレクトリ以下で 1GB 以上のファイルをリストアップして
```

変換操作を行うと、現在の入力行が次のようなコマンドに置き換わる。

```bash
find . -type f -size +1G -printf '%s %p\n' | sort -nr
```

生成されたコマンドは自動実行しない。利用者が内容を確認・編集し、通常の Enter で実行する。

## 2. 目的

- 覚えにくいオプションやパイプラインを自然言語から組み立てられるようにする。
- ターミナルから離れずにコマンドを生成できるようにする。
- Bash、Zsh、Fish で一貫した操作感を提供する。
- Codex CLI の既存認証とモデル設定を再利用する。
- AI が生成したコマンドを人間が確認してから実行する境界を維持する。

## 3. 非目標

- 自然言語の入力直後にコマンドを無確認で実行すること。
- Bash、Zsh、Fish の構文差を無視して、すべてを Bash コマンドとして生成すること。
- Codex CLI に代わる独自のモデルクライアントや認証機構を実装すること。
- 完全なシェル、ターミナルエミュレーター、常駐型エージェントを作ること。
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

キーバインドが利用できない環境向けに、明示的なコマンドも提供する。

```bash
hashai '1GB 以上のファイルを探して'
```

短縮エイリアスを提供する場合は、以下を候補とする。

```bash
?? '1GB 以上のファイルを探して'
```

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
5. コマンド、説明、危険度を呼び出し元へ返す。
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
- 明示的コマンドの結果を入力欄へ戻す用途では `print -z` も利用できる。

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

将来的には以下のプリセットを検討する。

| プリセット | 内容 |
|---|---|
| `minimal` | 補完または明示的コマンドのみ |
| `standard` | 明示的コマンドと短縮エイリアス |
| `full` | 上記に加えて `Ctrl+G` キーバインド |

## 7. Codex CLIとの連携

### 7.1 基本呼び出し

概念上の呼び出しは以下。

```bash
codex exec - \
  --ephemeral \
  --sandbox read-only \
  --skip-git-repo-check \
  --output-schema /path/to/schema.json
```

- `--ephemeral`: コマンド変換ごとのセッションを保存しない。
- `--sandbox read-only`: 生成処理中の書き込みを許可しない。
- `--skip-git-repo-check`: Gitリポジトリ外でも利用可能にする。
- `--output-schema`: 説明文やMarkdownが混ざらない構造化出力を要求する。
- 自然言語は引数展開ではなく標準入力で渡し、クォート問題を減らす。

実行時の作業ディレクトリは現在のディレクトリと一致させる。ただし、コマンド変換だけならCodexがディレクトリ内容を調査する必要はない。コンテキスト調査を許可するかは設定として分離する。

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
    "description": {
      "type": "string"
    },
    "risk": {
      "type": "string",
      "enum": ["safe", "review", "dangerous"]
    }
  },
  "required": ["command", "description", "risk"],
  "additionalProperties": false
}
```

通常の入力バッファには `command` だけを挿入する。`description` と `risk` は警告表示や説明操作に利用する。

### 7.3 プロンプト方針

プロンプトには最低限、次を含める。

- コマンドを実行せず、生成だけを行う。
- 対象OS、ディストリビューション、シェルを指定する。
- 現在の作業ディレクトリを指定する。
- 不要な `sudo` を使用しない。
- 破壊的操作や不可逆操作を避ける。
- 曖昧な値を捏造しない。
- コミットメッセージなどが不明なら、対話的なコマンドを選ぶ。
- 複数の操作は順序依存がある場合だけ `&&` で接続する。
- Bash向けの構文をFishへ出力しない。
- 構造化スキーマに従う。

シェル別のfew-shot例を用意し、OSやシェルに固有の慣用的なコマンドを誘導する。ただし例が生成結果を過度に固定しないよう、小さく保つ。

## 8. コンテキスト

### 8.1 デフォルトで送信する候補

- 対象シェルとバージョン
- OS、CPUアーキテクチャ、Linuxディストリビューション
- 現在の作業ディレクトリの論理パス
- Gitリポジトリ内かどうか
- 設定されたコマンド生成方針

### 8.2 オプトインにする候補

- 近隣ファイル名
- Git status
- プロジェクト種別
- package managerや代表的な設定ファイル
- 直前のコマンドと終了コード
- 直前の標準エラー出力

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

再生成や修正を提供する場合も会話セッションにはしない。元の依頼、直前の候補、今回の修正指示だけを一時的にCodexへ渡し、その処理が終了した時点で破棄する。

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

文字列ベースの検査は補助的な警告に留める。難読化、エイリアス、スクリプト経由などを完全には検出できないためである。

## 11. エラー処理

- Codexが見つからない場合は、インストール確認方法を表示する。
- 未認証の場合は、Codexのログイン方法を案内する。
- タイムアウト時は元の入力バッファを保持する。
- `Ctrl+C` でCodex子プロセスを終了できるようにする。
- 空または不正な構造化出力を入力バッファへ挿入しない。
- Coreの診断メッセージと生成コマンドの出力チャネルを分離する。
- 非TTY環境ではインライン操作を無効化し、非対話モードへ切り替える。
- 終了コードを定義し、シェル統合が失敗理由を判定できるようにする。

## 12. 設定

設定候補:

```toml
trigger = "# "
keybinding = "ctrl-g"
shell = "auto"
timeout_seconds = 30
context = "minimal"
insert_only = true
show_risk = true
reasoning_effort = "low"

[prompt]
extra_instructions = "Prefer rg over grep when available."
```

優先順位は、CLI引数、環境変数、プロジェクト設定、ユーザー設定、組み込みデフォルトの順を候補とする。APIキーをhashai自身の設定ファイルへ保存せず、Codex CLIの認証を利用する。

## 13. CLI案

```text
hashai <natural-language>
hashai generate <natural-language>
hashai explain <command>
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
- Codexの認証状態
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

### ShellGPT

- Execute / Describe / Abort の明確な選択肢。
- OSとシェルを考慮した生成。
- 生成コマンドを入力バッファへ戻して編集可能にする方式。
- コマンド生成と説明の分離。

### Warp AI

- `#` を自然言語モードとして認識させる操作モデル。
- 生成中・通常コマンド・自然言語入力を視覚的に区別する考え方。
- 履歴、再利用可能なプロンプト、AI候補の統合。

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
- multi-turn実装は参考対象に含めず、会話状態を持たない。

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

### Phase 4: コンテキスト拡張

- オプトインのプロジェクト検出。
- 直前の失敗コマンドを修正するモード。
- 再利用可能なプロンプト。

## 16. テスト方針

- Coreの単体テストではCodexプロセスを差し替え、正常、空出力、不正JSON、タイムアウト、キャンセルを検証する。
- Bash、Zsh、Fishごとに入力バッファ取得・置換を統合テストする。
- 空白、引用符、改行、日本語、絵文字を含む入力を検証する。
- 危険なコマンドが自動実行されないことをテストする。
- Gitリポジトリ内外の双方で動作を検証する。
- Linuxディストリビューション差とGNU/BSDコマンド差をテストケース化する。
- 生成品質は固定スナップショットだけで判定せず、スキーマ準拠と要求された性質を評価する。

代表的な評価入力:

```text
# 現在のディレクトリ以下で 1GB 以上のファイルをリストアップして
# 直近7日で変更されたJSONファイルをサイズ順に表示して
# git のコミットとプッシュを実行して
# 3000番ポートを使用しているプロセスを調べて
# このディレクトリを削除して
```

## 17. 未決事項

- Coreの実装言語。単一バイナリと起動速度を優先するならRustまたはGoが候補。
- 危険度をモデル出力だけでなく、ローカル解析でも補正するか。
- `#` + Enterの自動変換を提供するか、`Ctrl+G` のみにするか。
- GNUとBSDのコマンド差をどこまで自動判定するか。
- Codexのモデルと推論強度をhashai側で指定するか、Codex設定へ委譲するか。
- 説明表示を標準エラー、ポップアップ、pagerのどれで行うか。

初期判断としては、単一バイナリ、single-turn、`Ctrl+G`、入力バッファへの挿入のみ、Codex設定の再利用を優先する。

## 18. 参考資料

- [OpenAI: Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- [OpenAI: Codex developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
- [Deltik/shell-ai](https://github.com/Deltik/shell-ai)
- [TheR1D/shell_gpt](https://github.com/TheR1D/shell_gpt)
- [Warp AI](https://www.warp.dev/warp-ai)
- [Warp command entry](https://docs.warp.dev/terminal/entry)
- [matheusml/zsh-ai](https://github.com/matheusml/zsh-ai)
- [simonw/llm-cmd](https://github.com/simonw/llm-cmd)
- [microsoft/Codex-CLI](https://github.com/microsoft/Codex-CLI)
- [sigoden/aichat](https://github.com/sigoden/aichat)
