# Arquitetura

O marp-order é um binário Rust que sobe um servidor HTTP em `127.0.0.1` e serve uma
interface de página única. O conteúdo dos arquivos abertos fica no processo; o navegador
guarda, por aba, a ordem e as edições ainda não salvas e as envia ao salvar.

```
apresentacao.md ──► deck.rs (parse) ──► main.rs (estado + HTTP) ◄──► ui.html (navegador)
        ▲                                   │         │
        └──────── deck.rs (compose) ◄───────┘         ├──► thumbs.rs ──► marp-cli ──► JPEGs
                                                      └──► preview.rs ─► marp --watch ─► HTML
```

| Arquivo | Papel |
|---|---|
| `src/deck.rs` | Divide o arquivo em cabeçalho + lâminas e o remonta em outra ordem. Sem E/S. |
| `src/main.rs` | Linha de comando, estado, rotas HTTP, backup e gravação atômica. |
| `src/thumbs.rs` | Thread que chama o marp-cli e associa cada imagem a uma lâmina. |
| `src/links.rs` | Reescreve caminhos relativos de lâminas copiadas entre pastas. |
| `src/preview.rs` | Pré-visualização ao vivo do editor, com um `marp --watch` persistente. |
| `src/ui.html` | Interface (HTML, CSS e JavaScript puros), embutida no binário. |

Dependências: `tiny_http` e `serde_json`.

## Modelo de dados

```rust
App   { docs: Vec<Doc>, preview, .. }   // um Doc por arquivo aberto (aba)
Doc   { id, path, deck, disk_order, generation, thumb_dir, .. }
Deck  { source, head, slides }          // head = BOM + front-matter
Slide { lead, text, heading_start, lead_comment }
```

- `text` é o trecho exato do arquivo entre dois separadores.
- `lead` é a linha separadora que precedia a lâmina (`---`, `***`…), como estava escrita.
- `heading_start` indica lâmina aberta por um título de `headingDivider`, sem separador.

As lâminas são identificadas pelo índice que tinham quando o arquivo foi lido. O navegador
envia a lista de posições desejada, cada uma com o índice da lâmina e, se ela foi editada,
o texto novo (`Entry = (usize, Option<String>)`).

Salvar só uma reordenação não muda os índices. Salvar com edições é diferente: como o
texto novo pode criar ou fundir lâminas, o arquivo gravado é relido e vira a nova base
(`Doc::adopt`), com índices renumerados. As miniaturas das lâminas intactas são
reaproveitadas, e o navegador traduz o histórico de desfazer para a numeração nova
(`makeRemap`): cada item antigo vira a lâmina nova de mesma origem ou de mesmo texto, ou
uma entrada com texto próprio.

Cada gravação incrementa `save_seq`. O navegador manda o valor que conhecia; se outra
janela gravou no meio, o servidor responde 409 (`stale`) e a interface pergunta antes de
sobrescrever.

## Vários arquivos

Cada arquivo aberto é um `Doc` com um número (`doc`) que as rotas recebem na query string
ou no corpo do `POST`. As miniaturas de cada um ficam em uma pasta própria; o gerador
recebe por um canal o número do arquivo a renderizar. A pré-visualização do editor é uma
só (o marp-cli em modo `--watch` usa uma porta fixa) e é reiniciada quando o arquivo em
edição está em outra pasta.

`~/.config/marp-order/recent.json` guarda os últimos arquivos abertos e `session.json`,
os que estão abertos agora; sem argumentos, o programa reabre os da sessão.

A comparação é feita no navegador: as lâminas da aba atual (com as alterações pendentes)
são alinhadas com as do outro arquivo pela maior subsequência comum dos textos; nas
lacunas, lâminas de mesmo título ou com pelo menos metade das linhas em comum formam um
par "diferente". Uma lâmina copiada do outro arquivo entra na lista como uma entrada com
texto novo, igual a uma edição, depois de passar por `POST /api/import`, que acerta os
caminhos relativos de imagens quando os dois arquivos estão em pastas diferentes.

## Divisão das lâminas

O parser não é um parser Markdown completo: ele percorre o arquivo linha a linha com o
estado mínimo para decidir o que é separador. As regras reproduzem o comportamento do
[Marpit](https://github.com/marp-team/marpit), que divide o deck nas réguas horizontais
(`hr`) de nível superior do markdown-it.

Uma linha é **separador** se for uma régua (`---`, `***`, `___`, `- - -`; três ou mais
marcas, recuo de até 3 espaços) e **não** estiver em nenhum destes casos:

| Caso | Exemplo |
|---|---|
| Dentro de bloco de código cercado | ` ``` ` … `---` … ` ``` ` |
| Dentro de comentário, `<style>`, `<script>` ou `<pre>` | `<!--` … `---` … `-->` |
| Sublinhado de título setext | `Título` seguido de `---` |
| Aninhada em lista ou citação | `- item` / linha em branco / `  ---` |
| Recuo de 4 ou mais espaços (código indentado) | `    ---` |

O caso setext é o mais sutil: `---` logo abaixo de um parágrafo é um título de nível 2, mas
logo abaixo de um item de lista, de um título `#` ou de um comentário é separador.

### `headingDivider`

Lido do front-matter ou de um comentário (`<!-- headingDivider: 2 -->`); aceita um número
(níveis de 1 até N) ou uma lista (`[1, 3]`). Segue a regra do Marpit:

- Um título do nível configurado abre uma lâmina nova se **já houve conteúdo visível no
  deck**. Comentários não contam como conteúdo visível.
- Títulos setext (`===`, `---`) também dividem.
- Um `---` seguido de título divisor gera uma lâmina vazia entre os dois — o Marp se
  comporta assim, e o marp-order mostra essa lâmina.

## Remontagem (`Deck::compose`)

Recebe uma lista de entradas; os índices podem faltar (lâmina removida) ou repetir
(duplicada).

1. Ordem idêntica à original, sem edições, devolve `source` sem tocar em nada.
2. Se uma lâmina vem logo depois da mesma vizinha que tinha no arquivo, o par é copiado
   como estava, com o separador e o espaçamento originais.
3. Entre lâminas que não eram vizinhas, garante-se uma linha em branco antes do separador.
   Sem ela, `texto` seguido de `---` viraria título setext e as duas lâminas se fundiriam.
4. Lâminas `heading_start` não recebem separador: o título já divide.
5. O final do arquivo (com ou sem quebra de linha) é mantido como no original.
6. Uma lâmina editada recebe o texto novo entre as linhas em branco que cercavam o
   original, com as quebras de linha no estilo do arquivo (LF ou CRLF). Ela é sempre
   tratada como "não vizinha" (regra 3), e `heading_start` é recalculado sobre o texto novo.

## Miniaturas

`thumbs.rs` executa, na pasta do deck:

```
marp --no-stdin --allow-local-files --images jpeg --image-scale 0.5 -o <tmp>/s.jpeg deck.md
```

A k-ésima imagem corresponde à k-ésima lâmina gravada no disco. Se o número de imagens for
diferente do número de lâminas, o mapeamento é descartado e a interface alerta que a
divisão pode estar errada — é a rede de segurança contra casos que o parser não cobre.

Uma renderização é disparada ao abrir, ao recarregar e depois de cada salvamento. Se o
arquivo mudar durante a renderização, o resultado é descartado e ela é refeita.

Lâminas editadas ou acrescentadas ainda não estão no disco. Para elas a interface pede
`POST /api/draft` com o estado pendente; o servidor grava o rascunho em um arquivo oculto
ao lado do deck (`.<nome>.marp-order.draft.md`, apagado em seguida), renderiza e guarda
só as imagens pedidas, identificadas pela chave do texto (`d-<chave>.jpeg`).

## Pré-visualização do editor

Converter o deck inteiro a cada tecla com uma execução avulsa do marp-cli leva segundos,
quase tudo na partida do Node. Por isso `preview.rs` mantém um `marp --watch --template
bare` rodando sobre um rascunho (`draft.md`) em uma pasta temporária:

1. O navegador envia o estado atual (ordem + edições) e a posição da lâmina em edição.
2. O servidor monta o rascunho com `compose`, grava `draft.md` e espera o `draft.html`.
3. O editor carrega `/preview/slide.html?page=N` em um `<iframe>` oculto e troca pelo
   visível quando termina de carregar, para a lâmina não piscar.

O rascunho é o deck inteiro, não só a lâmina, para que estilos globais, numeração e
diretivas herdadas saiam como no resultado final; a página servida esconde as outras
lâminas por CSS. Detalhes que não são óbvios:

- **Marca de versão.** O marp-cli pode gravar o HTML de uma versão anterior logo depois da
  nossa escrita. Cada rascunho leva no fim um `<style>` com uma classe numerada, e o
  servidor só responde quando essa marca aparece no HTML.
- **Alterações perdidas.** O marp-cli ignora escritas feitas enquanto está iniciando; se a
  marca não aparece em 700 ms, o rascunho é regravado.
- **Recursos relativos.** O rascunho fica fora da pasta do deck, então `/preview/<caminho>`
  serve imagens, fontes e CSS a partir dela (somente tipos de mídia conhecidos, e nunca
  acima da pasta do deck).
- **Recarga automática.** O HTML do modo `--watch` traz um script que recarrega a página por
  websocket; ele é removido, e a resposta leva `Content-Security-Policy: connect-src 'none'`.
- **Processo órfão.** O marp-cli é iniciado por um pequeno script `sh` que o encerra quando
  a entrada padrão fecha, isto é, quando o marp-order termina, mesmo que seja morto.

O marp-cli em modo `--watch` abre um servidor websocket próprio na porta 37717.

## API HTTP

| Rota | Função |
|---|---|
| `GET /` | Interface |
| `GET /api/docs` | Arquivos abertos e lista de recentes |
| `GET /api/state?doc=N` | Lâminas (título, texto, avisos) e estado completo do arquivo |
| `GET /api/status?doc=N` | Consulta periódica: edição externa e situação das miniaturas |
| `GET /thumb/<doc>/<id>` | Miniatura da lâmina |
| `GET /preview/slide.html?page=N` | Lâmina N do rascunho em edição |
| `GET /preview/<caminho>` | Imagens e fontes da pasta do deck |
| `POST /api/open` | `{path}` — abre um arquivo em uma aba |
| `POST /api/close` | `{doc}` — fecha a aba |
| `POST /api/browse` | `{dir}` — pastas e arquivos Markdown, para o diálogo de abrir |
| `POST /api/save` | `{doc, generation, slides: [{id, text?}], force}` — grava ordem e edições |
| `POST /api/preview` | `{doc, generation, slides, index}` — converte o rascunho; devolve a página |
| `POST /api/draft` | `{doc, generation, slides}` — pede miniaturas das lâminas não salvas |
| `POST /api/head` | `{doc, head, force}` — grava um front-matter novo |
| `POST /api/import` | `{from, to, texts}` — acerta caminhos relativos de lâminas copiadas |
| `POST /api/reload` | `{doc}` — relê o arquivo do disco |
| `POST /api/rethumb` | `{doc}` — refaz as miniaturas |

O servidor recusa requisições cujo cabeçalho `Host` não seja `127.0.0.1` ou `localhost`
(contra DNS rebinding) e exige o cabeçalho `X-Marp-Order` nos `POST`, o que impede que
outra página aberta no navegador dispare uma gravação.

## Testes

`cargo test` roda os testes do parser (`deck.rs`), do servidor (`main.rs`, incluindo um
teste que sobe o servidor HTTP e faz pedidos de verdade) e de `links.rs`. No parser, o
principal padrão é o de ida e volta: remontar o deck em uma ordem, reler o resultado e
conferir que as lâminas saíram na ordem pedida.

A lógica da interface que não depende do navegador (alinhamento da comparação, diferença
por linha, notas do apresentador, realce, tradução do histórico) fica em um bloco
`<pure>` de `ui.html` e é testada com `node --test tests/ui_logic.test.js`.

`cargo test -- --ignored` roda `slide_count_matches_marp_cli`, que compara o número de
lâminas do parser com o do marp-cli em casos-limite. O GitHub Actions
(`.github/workflows/ci.yml`) roda tudo isso, mais o clippy, a cada push.
