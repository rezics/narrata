import { openBook, type Book, type BookView, type OpenBookOptions } from "@rezics/narrata";
import { IndexedDbStore, type CacheStore } from "@rezics/narrata/storage";

const execution = "execution:0190f2a0000070008000000000000001";
const manifest = new Uint8Array();
// @ts-expect-error A manifest cannot be opened without a chunk source.
const missingSource: OpenBookOptions = { manifest, execution };
// @ts-expect-error A program cannot specify both opening modes.
const mixed: OpenBookOptions = { manifest, pack: manifest, execution, fetchChunk: async () => manifest };
const open: (options: OpenBookOptions) => Promise<Book> = openBook;
const inspect: (book: Book) => Promise<BookView> = book => book.inspect();
const storage: Promise<CacheStore> = IndexedDbStore.open("type-example");
void [missingSource, mixed, open, inspect, storage];
