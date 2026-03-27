import Shelf from "./Shelf";

/**
 * Root component for the shelf Tauri window.
 * The shelf runs in its own native window (always-on-top dock).
 */
export default function ShelfWindow() {
  return (
    <div className="h-screen w-screen overflow-hidden bg-transparent">
      <Shelf />
    </div>
  );
}
