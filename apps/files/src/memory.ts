/**
 * A file system kept in memory, with the platform service's rules (crates/oxyos-files): the
 * application's model where there is no `OxyOSFiles` (the browser preview, the tests). Paths
 * outside the home and its volumes are refused, nothing replaces an item, and removing is the
 * Trash's alone.
 */
import {
  baseName,
  FileError,
  isWithin,
  joinPath,
  parentOf,
  validName,
  type FileEntry,
  type FileSystem,
  type ItemResult,
  type Places,
  type TrashedEntry,
  type WatchTarget,
} from "./model.ts";

interface Node {
  folder: boolean;
  size: number;
  modified: number;
}

/** What a fixture starts with: each path (from the home) a folder (ending in `/`) or a file of
 * that many bytes, modified at that time. */
export type Tree = Record<string, { size?: number; modified?: number } | null>;

export class MemoryFileSystem implements FileSystem {
  private nodes = new Map<string, Node>();
  private trashed = new Map<string, TrashedEntry>();
  private watchers = new Set<{ target: WatchTarget; changed: () => void }>();
  private clock: () => number;

  readonly home: string;
  private readonly volumes: { name: string; path: string }[];

  constructor(
    home: string,
    tree: Tree,
    volumes: { name: string; path: string }[] = [],
    clock: () => number = Date.now,
  ) {
    this.home = home;
    this.volumes = volumes;
    this.clock = clock;
    this.nodes.set(home, { folder: true, size: 0, modified: clock() });
    for (const volume of volumes) this.nodes.set(volume.path, { folder: true, size: 0, modified: clock() });
    for (const [relative, info] of Object.entries(tree)) {
      const folder = relative.endsWith("/");
      const path = relative.startsWith("/")
        ? relative.replace(/\/$/, "")
        : joinPath(home, relative.replace(/\/$/, ""));
      this.makeParents(path);
      this.nodes.set(path, {
        folder,
        size: folder ? 0 : (info?.size ?? 0),
        modified: info?.modified ?? clock(),
      });
    }
  }

  private makeParents(path: string) {
    for (let at = parentOf(path); !this.nodes.has(at) && at !== "/"; at = parentOf(at)) {
      this.nodes.set(at, { folder: true, size: 0, modified: this.clock() });
    }
  }

  private inScope(path: string): boolean {
    return [this.home, ...this.volumes.map((v) => v.path)].some((root) => isWithin(path, root));
  }

  private check(path: string): void {
    if (!path.startsWith("/") || path.split("/").some((part) => part === "." || part === "..")) {
      throw new FileError("refused", `${path} is not an absolute path`);
    }
    if (!this.inScope(path))
      throw new FileError("refused", `${path} is outside the home and the mounted volumes`);
  }

  private entry(path: string): FileEntry {
    const node = this.nodes.get(path);
    if (!node) throw new FileError("not-found", "nothing is there");
    const name = baseName(path);
    return {
      name,
      path,
      folder: node.folder,
      link: false,
      size: node.size,
      modified: node.modified,
      hidden: name.startsWith("."),
    };
  }

  private folderAt(path: string): void {
    this.check(path);
    const node = this.nodes.get(path);
    if (!node) throw new FileError("not-found", "nothing is there");
    if (!node.folder) throw new FileError("not-a-folder", "not a folder");
  }

  private item(path: string): void {
    this.check(path);
    if (path === this.home || this.volumes.some((v) => v.path === path)) {
      throw new FileError("refused", "the home and a volume's root stay where they are");
    }
    if (!this.nodes.has(path)) throw new FileError("not-found", "nothing is there");
  }

  /** Moves `from` and everything in it to `to`. */
  private relocate(from: string, to: string) {
    for (const [path, node] of [...this.nodes]) {
      if (isWithin(path, from)) {
        this.nodes.delete(path);
        this.nodes.set(to + path.slice(from.length), node);
      }
    }
  }

  private tell(path: string | null, trash = false) {
    for (const watcher of this.watchers) {
      const target = watcher.target;
      if (("trash" in target && trash) || ("path" in target && path !== null && target.path === path)) {
        watcher.changed();
      }
    }
  }

  async places(): Promise<Places> {
    const favourites = (["desktop", "documents", "downloads", "pictures", "music", "videos"] as const)
      .map((key) => ({ key, path: joinPath(this.home, FOLDERS[key]) }))
      .filter((favourite) => this.nodes.get(favourite.path)?.folder);
    return { home: this.home, favourites, volumes: this.volumes, trash: this.trashed.size };
  }

  async list(path: string) {
    this.folderAt(path);
    const entries = [...this.nodes.keys()]
      .filter((p) => p !== path && parentOf(p) === path)
      .map((p) => this.entry(p));
    return { path, entries };
  }

  async createFolder(parent: string, name: string) {
    this.folderAt(parent);
    if (!validName(name)) throw new FileError("invalid-name", "not a name an item can have");
    const path = joinPath(parent, name);
    if (this.nodes.has(path)) throw new FileError("exists", "an item with that name is already there");
    this.nodes.set(path, { folder: true, size: 0, modified: this.clock() });
    this.tell(parent);
    return this.entry(path);
  }

  async rename(path: string, name: string) {
    this.item(path);
    if (!validName(name)) throw new FileError("invalid-name", "not a name an item can have");
    const to = joinPath(parentOf(path), name);
    if (to !== path) {
      if (this.nodes.has(to)) throw new FileError("exists", "an item with that name is already there");
      this.relocate(path, to);
      this.tell(parentOf(path));
    }
    return this.entry(to);
  }

  async move(paths: readonly string[], to: string): Promise<ItemResult[]> {
    this.folderAt(to);
    return paths.map((path) => {
      try {
        this.item(path);
        if (isWithin(to, path)) throw new FileError("refused", "a folder cannot go inside itself");
        const target = joinPath(to, baseName(path));
        if (target !== path) {
          if (this.nodes.has(target))
            throw new FileError("exists", "an item with that name is already there");
          this.relocate(path, target);
          this.tell(parentOf(path));
          this.tell(to);
        }
        return { item: path, ok: true as const, to: target };
      } catch (error) {
        return { item: path, ok: false as const, error: (error as FileError).code };
      }
    });
  }

  async trash(paths: readonly string[]): Promise<ItemResult[]> {
    return paths.map((path) => {
      try {
        this.item(path);
        const entry = this.entry(path);
        let name = entry.name;
        for (let n = 2; [...this.trashed.values()].some((t) => baseName(t.id) === name); n++) {
          const dot = entry.name.lastIndexOf(".");
          name = dot > 0 ? `${entry.name.slice(0, dot)} ${n}${entry.name.slice(dot)}` : `${entry.name} ${n}`;
        }
        const id = joinPath(`${this.home}/.local/share/Trash/files`, name);
        for (const p of [...this.nodes.keys()]) if (isWithin(p, path)) this.nodes.delete(p);
        this.trashed.set(id, { ...entry, id, path: id, original: path, deleted: this.clock() });
        this.tell(parentOf(path));
        this.tell(null, true);
        return { item: path, ok: true as const, to: id };
      } catch (error) {
        return { item: path, ok: false as const, error: (error as FileError).code };
      }
    });
  }

  async trashList() {
    return [...this.trashed.values()];
  }

  async restore(ids: readonly string[]): Promise<ItemResult[]> {
    return ids.map((id) => {
      const item = this.trashed.get(id);
      if (!item?.original) return { item: id, ok: false as const, error: "not-found" as const };
      if (this.nodes.has(item.original)) return { item: id, ok: false as const, error: "exists" as const };
      this.trashed.delete(id);
      this.makeParents(item.original);
      this.nodes.set(item.original, { folder: item.folder, size: item.size, modified: item.modified });
      this.tell(parentOf(item.original));
      this.tell(null, true);
      return { item: id, ok: true as const, to: item.original };
    });
  }

  async deleteFromTrash(ids: readonly string[]): Promise<ItemResult[]> {
    return ids.map((id) => {
      if (!this.trashed.delete(id)) return { item: id, ok: false as const, error: "refused" as const };
      this.tell(null, true);
      return { item: id, ok: true as const, to: id };
    });
  }

  async emptyTrash() {
    const count = this.trashed.size;
    this.trashed.clear();
    this.tell(null, true);
    return count;
  }

  async watch(target: WatchTarget, changed: () => void) {
    if ("path" in target) this.folderAt(target.path);
    const watcher = { target, changed };
    this.watchers.add(watcher);
    return () => {
      this.watchers.delete(watcher);
    };
  }
}

const FOLDERS = {
  desktop: "Desktop",
  documents: "Documents",
  downloads: "Downloads",
  pictures: "Pictures",
  music: "Music",
  videos: "Videos",
} as const;
