import { useMemo } from "react";
import { motion } from "framer-motion";
import { useNotificationStore } from "../../stores/notificationStore";
import { appIcons, settingsIcon } from "./appIcons";

interface NotificationPanelProps {
  onClose: () => void;
}

function timeAgo(timestamp: number): string {
  const diff = Math.floor((Date.now() - timestamp) / 1000);
  if (diff < 60) return "now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function NotificationPanel({ onClose }: NotificationPanelProps) {
  const notifications = useNotificationStore((s) => s.notifications);
  const dismiss = useNotificationStore((s) => s.dismiss);
  const clearApp = useNotificationStore((s) => s.clearApp);
  const clearAll = useNotificationStore((s) => s.clearAll);

  const grouped = useMemo(() => {
    const map = new Map<string, { app: string; notifications: typeof notifications }>();
    for (const n of notifications) {
      const existing = map.get(n.app);
      if (existing) {
        existing.notifications.push(n);
      } else {
        map.set(n.app, { app: n.app, notifications: [n] });
      }
    }
    return Array.from(map.values());
  }, [notifications]);

  const hasNotifications = grouped.length > 0;

  return (
    <>
      {/* Click-away backdrop */}
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, y: 8, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 8, scale: 0.97 }}
        transition={{ duration: 0.22, ease: [0.2, 0, 0, 1] }}
        className="fixed right-2 bottom-[64px] z-50 flex w-[380px] origin-bottom-right flex-col rounded-[20px] border border-white/20 bg-white/12 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]"
        style={{ maxHeight: "calc(100vh - 100px)" }}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 pt-4 pb-2">
          <h2 className="text-[13px] font-semibold tracking-wide text-white/90 uppercase">
            Notifications
          </h2>
          {hasNotifications && (
            <button
              onClick={clearAll}
              className="cursor-pointer text-[12px] font-medium text-white/50 transition-colors hover:text-white/80"
            >
              Clear All
            </button>
          )}
        </div>

        {/* Notification list */}
        <div className="flex-1 overflow-y-auto overflow-x-hidden px-3 pt-1 pb-3 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {hasNotifications ? (
            <div className="flex flex-col gap-2.5">
              {grouped.map((group) => (
                <div key={group.app} className="flex flex-col gap-[3px]">
                  {/* Group header */}
                  <div className="flex items-center justify-between px-1.5 pb-0.5">
                    <div className="flex items-center gap-2">
                      <img
                        src={appIcons[group.app] ?? settingsIcon}
                        alt={group.app}
                        className="h-4 w-4 rounded-[4px] object-cover"
                        draggable={false}
                      />
                      <span className="text-[11px] font-semibold tracking-wide text-white/60 uppercase">
                        {group.app}
                      </span>
                    </div>
                    {group.notifications.length > 1 && (
                      <button
                        onClick={() => clearApp(group.app)}
                        className="cursor-pointer text-[11px] text-white/40 transition-colors hover:text-white/70"
                      >
                        Clear
                      </button>
                    )}
                  </div>

                  {/* Notification cards */}
                  {group.notifications.map((notification, i) => (
                    <motion.div
                      key={notification.id}
                      layout
                      initial={{ opacity: 0, x: 20 }}
                      animate={{ opacity: 1, x: 0 }}
                      exit={{ opacity: 0, x: -40 }}
                      transition={{
                        duration: 0.2,
                        delay: i * 0.03,
                        ease: [0.2, 0, 0, 1],
                      }}
                      className="group relative overflow-hidden rounded-2xl border border-white/10 bg-white/8 p-3.5 transition-colors duration-150 hover:bg-white/12"
                    >
                      {/* Dismiss button */}
                      <button
                        onClick={() => dismiss(notification.id)}
                        className="absolute top-2 right-2.5 flex h-5 w-5 cursor-pointer items-center justify-center rounded-full bg-white/10 text-white/40 opacity-0 transition-all duration-150 hover:bg-white/20 hover:text-white/70 group-hover:opacity-100"
                        aria-label="Dismiss"
                      >
                        <svg width="8" height="8" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                          <path d="M1 1l8 8M9 1l-8 8" />
                        </svg>
                      </button>

                      <div className="flex items-start gap-3">
                        {/* App icon */}
                        <img
                          src={appIcons[notification.app] ?? settingsIcon}
                          alt={notification.app}
                          className="mt-0.5 h-8 w-8 shrink-0 rounded-lg object-cover"
                          draggable={false}
                        />

                        <div className="min-w-0 flex-1">
                          {/* Title + time */}
                          <div className="flex items-baseline justify-between gap-2">
                            <p className="truncate text-[13px] font-semibold leading-tight text-white/90">
                              {notification.title}
                            </p>
                            <span className="shrink-0 text-[10px] text-white/35">
                              {timeAgo(notification.timestamp)}
                            </span>
                          </div>
                          {/* Body */}
                          <p className="mt-0.5 line-clamp-2 text-[12px] leading-snug text-white/55">
                            {notification.body}
                          </p>
                        </div>
                      </div>
                    </motion.div>
                  ))}
                </div>
              ))}
            </div>
          ) : (
            /* Empty state */
            <div className="flex flex-col items-center justify-center py-16">
              <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-white/8">
                <svg viewBox="0 0 24 24" width="22" height="22" fill="white" opacity="0.3">
                  <path d="M19.993 9.042C19.48 5.017 16.054 2 11.996 2s-7.49 3.021-7.999 7.051L2.866 18H7.1c.463 2.282 2.481 4 4.9 4s4.437-1.718 4.9-4h4.236l-1.143-8.958zM12 20c-1.306 0-2.417-.835-2.829-2h5.658c-.412 1.165-1.523 2-2.829 2zm-6.866-4l.847-6.698C6.364 6.272 8.941 4 11.996 4s5.627 2.268 6.013 5.295L18.864 16H5.134z" />
                </svg>
              </div>
              <p className="text-[13px] font-medium text-white/40">
                No Notifications
              </p>
            </div>
          )}
        </div>
      </motion.div>
    </>
  );
}
