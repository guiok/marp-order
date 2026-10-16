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
- **Interface em português e inglês**, com troca a qualquer momento.
- **Funciona ao lado de agentes de IA** que editam o arquivo `.md`: veja
  [Escrever slides com um agente de IA](#escrever-slides-com-um-agente-de-ia).

> Os nomes de botões citados abaixo são os da interface em inglês; em português eles
> aparecem traduzidos (**Discard** é **Descartar**, **Compare…** é **Comparar…** etc.).

## Requisitos

| Para | Precisa de |
|---|---|
| Compilar do código-fonte | [Rust](https://rustup.rs) 1.80 ou mais novo |
| Miniaturas e pré-visualização (opcional) | [marp-cli](https://github.com/marp-team/marp-cli) e Chrome, Chromium ou Edge |

Sem o marp-cli o programa funciona do mesmo jeito, mostrando o texto de cada lâmina no
lugar da miniatura; o editor abre, mas sem a pré-visualização.

O marp-order é desenvolvido, usado e testado à mão somente no Linux. O código tem suporte
a macOS e Windows, e os testes automáticos rodam neles, mas ninguém usou o programa nesses
sistemas, por isso não há binários publicados para eles.

## Instalação

Cada [release](https://github.com/guiok/marp-order/releases) traz dois arquivos para Linux
(x86_64):

- **Pacote `.deb`**, para Debian, Ubuntu e derivados. Baixe e rode
  `sudo apt install ./marp-order_<versão>_amd64.deb`. Ele instala `/usr/bin/marp-order` e
  uma página de manual, e `sudo apt remove marp-order` os remove. É um arquivo avulso, não
  um repositório apt, então versões novas precisam ser baixadas de novo.
- **Arquivo `.tar.gz`** só com o binário: descompacte e ponha o `marp-order` em alguma
  pasta do PATH.

**Do código-fonte**, com o Rust instalado (o único caminho em outros sistemas, sem teste):

```bash
cargo install --git https://github.com/guiok/marp-order
```

Isso instala o binário em `~/.cargo/bin`. Dentro de um clone do repositório,
`cargo install --path .` faz o mesmo e `make install` copia para `~/.local/bin`.

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
| Copiar / recortar / colar, também entre abas | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Copiar para outro arquivo | arrastar os cartões para a aba dele |
| Buscar lâminas por texto | `Ctrl+F`, depois `Enter` para o próximo resultado |
| Desfazer / refazer | `Ctrl+Z` / `Ctrl+Y` |
| Editar o conteúdo | duplo clique |
| Descartar alterações | botão **Discard** |
| Alternar entre português e inglês | seletor **EN / PT**, no canto superior direito |
| Salvar no arquivo | `Ctrl+S` |
| Salvar em outro arquivo | **Save as…** ou `Ctrl+Shift+S` |
| Recarregar o arquivo do disco | `Ctrl+R`, ou sozinho com **Auto-reload** ligado |
| Abrir outro arquivo | `Ctrl+O` ou **+ Open** |
| Editar o front-matter | botão **Front matter** |
| Apresentar com narração | botão **▶ Present** |
| Comparar com outro arquivo aberto | botão **Compare…** |

Cada cartão mostra a posição nova e, se a lâmina mudou de lugar, a posição que ela tem no
arquivo (`was 8`).

### Editar uma lâmina

Duplo clique em um cartão abre o editor: o Markdown da lâmina à esquerda, com realce de
sintaxe, e à direita a lâmina renderizada pelo Marp com o tema, as imagens e as diretivas
do deck. A pré-visualização é refeita a cada alteração (algumas centenas de milissegundos).

As notas do apresentador (comentários HTML que não são diretivas do Marp) ficam em uma
caixa própria abaixo do texto. Se você alterar o texto ou as notas, elas são regravadas
como um único comentário no fim da lâmina.

| Ação | Como |
|---|---|
| Aplicar a edição | `Ctrl+Enter` ou **Apply** |
| Aplicar e gravar o arquivo | `Ctrl+S` |
| Aplicar e editar a lâmina anterior / seguinte | `Alt+←` / `Alt+→`, ou os botões **◀ ▶** |
| Remover a lâmina e editar a seguinte | `Alt+Delete` ou **Remove slide** |
| Cancelar | `Esc` ou **Cancel** |

Aplicar não grava o arquivo: a edição fica pendente, como uma reordenação, e pode ser
desfeita com `Ctrl+Z`. O cartão ganha a etiqueta **edited**, e a miniatura dele é
refeita em segundo plano alguns segundos depois. Escrever `---` dentro de uma lâmina a divide em duas; as
novas lâminas aparecem na matriz depois de salvar.

O botão **Front matter** abre o mesmo editor no bloco YAML do início do arquivo (tema,
`paginate`, `headingDivider`…), com a primeira lâmina como pré-visualização. Ele vale
para o arquivo todo, então é gravado no disco assim que você salva ali, e só fica
disponível quando a aba não tem alterações pendentes.

### Apresentar com narração

**▶ Present** mostra as lâminas em tela cheia, a partir da selecionada, exatamente como
estão na aba (alterações pendentes incluídas). `→`, `Espaço` ou `PageDown` avançam, `←` ou
`PageUp` voltam, `Home` e `End` vão para as pontas, e `Esc` fecha.

Durante a apresentação, as **notas do apresentador** de cada lâmina — os comentários HTML
que não são diretivas do Marp — aparecem abaixo dela e são lidas em voz alta pelo
conversor de texto em fala do navegador:

- A lista de vozes põe primeiro as do idioma do deck, tirado de `lang:` no front-matter
  (`lang: pt-BR`), ou do idioma da interface quando o deck não tem. Você pode escolher
  qualquer outra voz; a escolha é lembrada.
- **Advance when the notes end** transforma o deck em uma apresentação que roda sozinha:
  cada lâmina avança quando as notas terminam, ou depois de quatro segundos se não tiver
  notas.
- **Speed** muda a velocidade da leitura, e desmarcar **Read the notes aloud** a silencia.

As vozes vêm do seu navegador e do sistema operacional, não do marp-order, então os
idiomas disponíveis e a qualidade dependem deles. Alguns navegadores usam vozes online, que
enviam as notas ao fabricante do navegador. Precisa do marp-cli, como a pré-visualização do
editor.

### Vários arquivos

Cada arquivo fica em uma aba. **+ Open** (`Ctrl+O`) mostra os arquivos abertos
recentemente e um navegador de pastas restrito a `.md` e `.markdown`; o **×** fecha a
aba. As alterações não salvas, o histórico de desfazer e a seleção são mantidos por aba
enquanto você alterna, e um ponto na aba indica alterações não salvas. Lâminas podem ser
copiadas entre abas com `Ctrl+C`/`Ctrl+V` ou arrastando-as para outra aba, onde entram
no fim.

A lista de arquivos abertos é lembrada: rodar `marp-order` sem arquivo reabre as abas da
última sessão. Os recentes e a sessão ficam em `~/.config/marp-order/` (ou
`$XDG_CONFIG_HOME/marp-order/`; `%APPDATA%\marp-order` no Windows).

### Comparar e mesclar

![Comparação de dois decks](docs/compare.png)

Com dois ou mais arquivos abertos, **Compare…** alinha as lâminas da aba atual com as de
outro arquivo aberto (como está salvo no disco) e marca cada diferença:

| Diferença | O que dá para fazer |
|---|---|
| **different** – mesmo título ou texto quase igual | **← Use other** troca a sua lâmina pela do outro; *Show what differs* marca as linhas alteradas |
| **only in the other** | **← Add** copia a lâmina para o seu arquivo, naquela posição |
| **only here** | **Remove** apaga a lâmina do seu arquivo |

**Take all from other** aplica tudo de uma vez. Cada ação vira uma alteração pendente na
aba atual, como uma edição manual: pode ser desfeita, e nada é gravado até você salvar. O
outro arquivo nunca é modificado. Uma diferença no front-matter (tema, diretivas
globais) é mostrada, mas não mesclada: edite-o com o botão **Front matter**.

Quando uma lâmina vem de um arquivo em outra pasta — pela comparação, colando ou
arrastando — os caminhos relativos de imagens que existem lá são reescritos (`img/a.png`
vira `../outra/img/a.png`), para a lâmina continuar apontando para os mesmos arquivos.
Nada é copiado.

### Opções

```
marp-order [opções] [apresentacao.md ...] [-- argumentos do marp-cli]

  --port <n>         porta do servidor local (padrão: escolhida pelo sistema)
  --no-open          não abre o navegador automaticamente
  --no-thumbs        não gera miniaturas (mostra só o texto das lâminas)
  --no-cache         não guarda miniaturas em disco entre execuções
  --marp-cmd <cmd>   comando do marp-cli (padrão: `marp` ou `npx @marp-team/marp-cli`)
  -h, --help         mostra a ajuda
  -V, --version      mostra a versão
```

O que vier depois de `--` é repassado ao marp-cli. Use para temas próprios:

```bash
marp-order aula.md -- --theme-set tema.css
```

O comando do marp-cli também pode ser definido pela variável `MARP_ORDER_MARP_CMD`.

## Escrever slides com um agente de IA

O marp-order não tem IA embutida, e não precisa: um deck é um arquivo `.md` comum, então
qualquer agente de código que edite arquivos — Claude Code, GitHub Copilot, Gemini CLI,
Codex e similares — pode escrever e revisar as lâminas enquanto o marp-order mostra o
resultado.

1. Abra o deck no marp-order e, em um terminal ou editor ao lado, inicie o seu agente na
   pasta do deck.
2. Peça o que quer em termos do arquivo: *"Em `aula.md`, reduza a lâmina 7 a três tópicos
   e acrescente notas do apresentador em toda lâmina que não tiver."*
3. Quando o agente grava o arquivo, o marp-order o recarrega sozinho em um ou dois
   segundos e mostra as lâminas novas. Isso exige a opção **Auto-reload** (canto superior
   direito) ligada, que é o padrão, e nenhuma alteração não salva na aba; caso contrário
   aparece o aviso **O arquivo foi alterado fora do marp-order** e o `Ctrl+R` recarrega.
4. Reorganize, acerte detalhes no editor e salve como de costume.

O que faz isso funcionar bem:

- **Um escritor por vez.** Salve (ou descarte) as suas alterações pendentes antes de pedir
  ao agente para editar. Se o arquivo mudar enquanto você tem alterações não salvas, o
  marp-order bloqueia o salvamento até você escolher entre recarregar e sobrescrever.
- **Revise em vez de confiar.** Peça ao agente para gravar em uma cópia (`aula-ia.md`),
  abra os dois arquivos e use **Compare…** para ver o que mudou lâmina a lâmina e
  aproveitar só o que quiser.
- **Conte ao agente as regras do deck.** Ajuda dizer que as lâminas são separadas por
  `---`, que o front-matter do início precisa ficar, e que comentários como
  `<!-- _class: lead -->` são diretivas do Marp, não notas.
- **Sempre dá para voltar.** O marp-order guarda um backup antes da primeira gravação de
  cada execução (veja abaixo); para as edições do próprio agente, mantenha o deck sob
  controle de versão.

Lembre que o agente envia o conteúdo das suas lâminas ao fornecedor dele.

## O que acontece com o seu arquivo

- **Front-matter fixo.** O bloco YAML inicial nunca é movido.
- **Lâminas intactas.** O texto de cada lâmina não editada — incluindo notas do
  apresentador e diretivas como `<!-- _class: lead -->` — é copiado byte a byte. Lâminas que continuam
  vizinhas mantêm até o espaçamento entre elas, então o `git diff` mostra só o que mudou.
- **Ida e volta exata.** Voltar à ordem original devolve o arquivo idêntico.
- **Backup.** Antes da primeira gravação de cada execução, o arquivo é copiado para
  `.marp-order-backups/apresentacao.md.<data>.bak`, ao lado dele. Ficam guardadas as dez
  cópias mais recentes de cada arquivo.
- **Salvar como.** **Save as…** grava a aba, com as alterações pendentes, em outro arquivo, e
  a aba passa a mostrá-lo; o original fica como estava no disco. Salvar em outra pasta
  reescreve os caminhos relativos de imagens para continuarem achando os mesmos arquivos.
  Substituir um arquivo existente pede confirmação e guarda um backup dele.
- **Gravação atômica.** O conteúdo vai para um arquivo temporário e é renomeado, de modo
  que uma queda no meio não deixa o deck pela metade.
- **Edição externa.** Se o arquivo mudar no seu editor enquanto o marp-order está aberto e
  a aba não tiver alterações não salvas, ele é recarregado sozinho (a caixa **Auto-reload**,
  no canto superior direito, desliga isso). Com alterações não salvas nada é recarregado, e
  o salvamento é bloqueado até você escolher entre recarregar ou sobrescrever. O mesmo
  acontece quando o arquivo foi salvo por outra janela do marp-order.
- **Cache de miniaturas.** As miniaturas dos últimos 40 decks ficam em
  `~/.cache/marp-order/` (`%LOCALAPPDATA%\marp-order` no Windows), de modo que um
  arquivo que não mudou mostra as lâminas na hora ao ser reaberto. Elas são refeitas em
  segundo plano mesmo assim, para o caso de uma imagem ou o tema ter mudado. `--no-cache`
  desliga isso.
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
- **Recursos da pré-visualização.** Imagens e fontes precisam estar na pasta do deck ou
  abaixo dela; caminhos com `../` ou absolutos não aparecem no editor (as miniaturas e o
  resultado final não são afetados).
- **Uso local.** O servidor aceita apenas conexões de `127.0.0.1` e não tem autenticação;
  não foi feito para ser exposto na rede.
- **Porta da pré-visualização.** Enquanto a pré-visualização do editor está em uso, o
  modo watch do marp-cli abre um servidor WebSocket próprio para avisos de recarga, na
  primeira porta livre a partir da 37717, em todas as interfaces de rede. O marp-cli não
  tem opção para mudar isso. O marp-order não usa esse servidor e nenhum conteúdo do deck
  passa por ele.

## Desenvolvimento

```bash
make test     # testes do parser, do servidor e da interface
make build    # binário de release em target/<sistema>-<cpu>/release/, ex.: target/Linux-x86_64/
make                          # executa, reabrindo a última sessão
make run DECK=apresentacao.md
```

A estrutura do código e as regras de divisão de lâminas estão descritas em
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (em inglês). Contribuições são bem-vindas: ao mexer no
parser, acrescente um teste em `src/deck.rs` e um caso em `slide_count_matches_marp_cli`,
que compara o número de lâminas com o marp-cli de verdade (`cargo test -- --ignored`).
As mesmas verificações rodam a cada push no GitHub Actions, em Linux, macOS e Windows.
Os textos da interface passam por `t('…')`; um texto novo precisa da tradução na tabela
`PT` de `src/ui.html`.

## Licença

[MIT](LICENSE)
