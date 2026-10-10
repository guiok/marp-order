# marp-order

Reorganize as lâminas de uma apresentação [Marp](https://marp.app) arrastando miniaturas
em uma matriz, edite o conteúdo de cada uma com pré-visualização ao vivo — e o arquivo
`.md` é reescrito com o resultado.

[English](README.md) · **Português**

![Demonstração do marp-order](docs/demo.gif)

Reordenar um deck Marp à mão significa recortar e colar blocos de Markdown entre linhas
`---`, sem ver o resultado. O `marp-order` mostra o deck inteiro de uma vez, como o
"classificador de slides" dos editores de apresentação, e cuida da reescrita do arquivo
sem alterar o conteúdo das lâminas.

- **Um binário só**, escrito em Rust; a interface roda no seu navegador, em `localhost`.
- **Miniaturas reais**, renderizadas pelo marp-cli com o tema da sua apresentação.
- **Editor com pré-visualização ao vivo**: duplo clique abre o Markdown da lâmina e o
  resultado é redesenhado enquanto você digita.
- **Vários arquivos em abas**, com lista de recentes e a última sessão restaurada.
- **Comparação e merge**: veja quais lâminas diferem entre dois decks e copie de um para o outro.
- **Não reformata nada**: lâminas não editadas são movidas byte a byte; o front-matter
  fica onde está.
- **Seguro**: backup automático, gravação atômica e detecção de edição externa.

> A interface do programa é em inglês.

## Requisitos

| Para | Precisa de |
|---|---|
| Compilar | [Rust](https://rustup.rs) 1.80 ou mais novo |
| Miniaturas e pré-visualização (opcional) | [marp-cli](https://github.com/marp-team/marp-cli) e Chrome, Chromium ou Edge |

Sem o marp-cli o programa funciona do mesmo jeito, mostrando o texto de cada lâmina no
lugar da miniatura; o editor abre, mas sem a pré-visualização. O navegador é aberto com `xdg-open` (Linux); em outros sistemas, abra
manualmente o endereço mostrado no terminal.

## Instalação

```bash
git clone https://github.com/guiok/marp-order.git
cd marp-order
cargo install --path .
```

Isso instala o binário em `~/.cargo/bin`. Como alternativa, `make install` compila e copia
para `~/.local/bin`.

Para as miniaturas:

```bash
npm install -g @marp-team/marp-cli
```

Se `marp` não estiver no PATH, o programa tenta `npx @marp-team/marp-cli`, que baixa o
pacote na primeira execução.

## Uso

```bash
marp-order apresentacao.md            # um arquivo
marp-order aula.md oficina.md         # cada arquivo em uma aba
marp-order                            # os arquivos abertos da última vez
```

O navegador abre com a matriz. Reorganize, salve com `Ctrl+S` e encerre o programa com
`Ctrl+C` no terminal.

| Ação | Como |
|---|---|
| Reordenar | arrastar o cartão (ou vários selecionados) |
| Selecionar várias | `Ctrl`+clique, `Shift`+clique, `Ctrl+A` |
| Mover pelo teclado | `Alt+←` / `Alt+→`, `Alt+Home` / `Alt+End` |
| Duplicar / remover | `Ctrl+D` / `Delete` |
| Desfazer / refazer | `Ctrl+Z` / `Ctrl+Y` |
| Editar o conteúdo | duplo clique |
| Descartar alterações | botão **Discard** |
| Salvar no arquivo | `Ctrl+S` |
| Recarregar o arquivo do disco | `Ctrl+R` |
| Abrir outro arquivo | `Ctrl+O` ou **+ Open** |
| Comparar com outro arquivo aberto | botão **Compare…** |

Cada cartão mostra a posição nova e, se a lâmina mudou de lugar, a posição que ela tem no
arquivo (`was 8`).

### Editar uma lâmina

Duplo clique em um cartão abre o editor: o Markdown da lâmina à esquerda e, à direita, a
lâmina renderizada pelo Marp com o tema, as imagens e as diretivas do deck. A
pré-visualização é refeita a cada alteração (algumas centenas de milissegundos).

| Ação | Como |
|---|---|
| Aplicar a edição | `Ctrl+Enter` ou **Apply** |
| Aplicar e gravar o arquivo | `Ctrl+S` |
| Cancelar | `Esc` ou **Cancel** |

Aplicar não grava o arquivo: a edição fica pendente, como uma reordenação, e pode ser
desfeita com `Ctrl+Z`. O cartão ganha a etiqueta **edited**, e a miniatura dele é
atualizada quando você salva. Escrever `---` dentro de uma lâmina a divide em duas; as
novas lâminas aparecem na matriz depois de salvar.

### Vários arquivos

Cada arquivo fica em uma aba. **+ Open** (`Ctrl+O`) mostra os arquivos abertos
recentemente e um navegador de pastas restrito a `.md` e `.markdown`; o **×** fecha a
aba. As alterações não salvas, o histórico de desfazer e a seleção são mantidos por aba
enquanto você alterna, e um ponto na aba indica alterações não salvas.

A lista de arquivos abertos é lembrada: rodar `marp-order` sem arquivo reabre as abas da
última sessão. Os recentes e a sessão ficam em `~/.config/marp-order/` (ou
`$XDG_CONFIG_HOME/marp-order/`).

### Comparar e mesclar

![Comparação de dois decks](docs/compare.png)

Com dois ou mais arquivos abertos, **Compare…** alinha as lâminas da aba atual com as de
outro arquivo aberto (como está salvo no disco) e marca cada diferença:

| Diferença | O que dá para fazer |
|---|---|
| **different** – mesmo título, conteúdo diferente | **← Use other** troca a sua lâmina pela do outro; *Show Markdown* mostra os dois textos |
| **only in the other** | **← Add** copia a lâmina para o seu arquivo, naquela posição |
| **only here** | **Remove** apaga a lâmina do seu arquivo |

**Take all from other** aplica tudo de uma vez. Cada ação vira uma alteração pendente na
aba atual, como uma edição manual: pode ser desfeita, e nada é gravado até você salvar. O
outro arquivo nunca é modificado. Só as lâminas são comparadas; o front-matter (tema,
diretivas globais) não é mesclado, e as imagens referenciadas por uma lâmina copiada não
são copiadas.

### Opções

```
marp-order [opções] [apresentacao.md ...] [-- argumentos do marp-cli]

  --port <n>         porta do servidor local (padrão: escolhida pelo sistema)
  --no-open          não abre o navegador automaticamente
  --no-thumbs        não gera miniaturas (mostra só o texto das lâminas)
  --marp-cmd <cmd>   comando do marp-cli (padrão: `marp` ou `npx @marp-team/marp-cli`)
  -h, --help         mostra a ajuda
  -V, --version      mostra a versão
```

O que vier depois de `--` é repassado ao marp-cli. Use para temas próprios:

```bash
marp-order aula.md -- --theme-set tema.css
```

O comando do marp-cli também pode ser definido pela variável `MARP_ORDER_MARP_CMD`.

## O que acontece com o seu arquivo

- **Front-matter fixo.** O bloco YAML inicial nunca é movido.
- **Lâminas intactas.** O texto de cada lâmina não editada — incluindo notas do
  apresentador e diretivas como `<!-- _class: lead -->` — é copiado byte a byte. Lâminas que continuam
  vizinhas mantêm até o espaçamento entre elas, então o `git diff` mostra só o que mudou.
- **Ida e volta exata.** Voltar à ordem original devolve o arquivo idêntico.
- **Backup.** Antes da primeira gravação é criado `apresentacao.md.bak` (ou `.bak1`,
  `.bak2`… se já existir).
- **Gravação atômica.** O conteúdo vai para um arquivo temporário e é renomeado, de modo
  que uma queda no meio não deixa o deck pela metade.
- **Edição externa.** Se o arquivo mudar no seu editor enquanto o marp-order está aberto,
  o salvamento é bloqueado até você escolher entre recarregar ou sobrescrever.
- **Conferência com o Marp.** As miniaturas são refeitas a cada salvamento; se o Marp
  enxergar um número de lâminas diferente do marp-order, aparece um alerta.

### O aviso ⚠

No Marp, uma diretiva local escrita **sem** `_` vale da lâmina em que aparece até o fim:

```markdown
<!-- paginate: true -->   ← esta lâmina e todas as seguintes
<!-- _class: lead -->     ← só esta lâmina
```

Mover uma lâmina com `paginate`, `class`, `header`, `footer`, `backgroundColor` etc. muda
quais lâminas são afetadas. O marp-order não altera essas diretivas; ele marca o cartão
com ⚠ para você conferir.

## Limitações

- **`headingDivider` e comentários.** Com `headingDivider`, o Marp não permite comentário
  antes do título de uma lâmina que não seja a primeira: ele fica na lâmina anterior. Se a
  primeira lâmina do deck tem um comentário assim e é movida, o comentário muda de lâmina.
  O cartão é marcado com ⚠.
- **Codificação.** O arquivo precisa estar em UTF-8.
- **Histórico de desfazer.** Salvar edições de conteúdo relê o arquivo e reinicia o
  histórico de desfazer/refazer. Salvar só reordenações preserva o histórico.
- **Recursos da pré-visualização.** Imagens e fontes precisam estar na pasta do deck ou
  abaixo dela; caminhos com `../` ou absolutos não aparecem no editor (as miniaturas e o
  resultado final não são afetados).
- **Front-matter.** O editor altera lâminas; o front-matter é editado no seu editor de texto.
- **Uso local.** O servidor aceita apenas conexões de `127.0.0.1` e não tem autenticação;
  não foi feito para ser exposto na rede.

## Desenvolvimento

```bash
make test     # testes do parser e do writer
make build    # binário de release em target/release/
make                          # executa, reabrindo a última sessão
make abrir DECK=apresentacao.md
```

A estrutura do código e as regras de divisão de lâminas estão descritas em
[docs/ARQUITETURA.md](docs/ARQUITETURA.md). Contribuições são bem-vindas: ao mexer no
parser, acrescente um teste em `src/deck.rs` e confira o caso contra o marp-cli.

## Licença

[MIT](LICENSE)
