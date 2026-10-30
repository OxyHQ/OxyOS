import React from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Box, PressArea, Scroll } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication, ShellNotifications } from "../model";
import { groupNotifications } from "../notifications";
import { BANNER_WIDTH, NotificationCard } from "./NotificationBanner";

/**
 * The Notification Center (docs/adr/0026-notifications.mdx), opened from the top bar's clock as a
 * Mac's: the session's notifications at the screen's right, one group per application (its icon
 * and name, the newest first), each pressed to open it in its application or removed; the whole
 * of an application's cleared at once. Do Not Disturb is said at its top. A press beside it closes
 * it.
 */
export function NotificationCenter({
  notifications,
  applications,
  now,
  top,
  close,
}: {
  notifications: ShellNotifications;
  applications: readonly ShellApplication[];
  now: Date;
  /** Where it begins: below the top bar. */
  top: number;
  close: () => void;
}) {
  const groups = groupNotifications(
    notifications.list.filter((notification) => !notification.withheld),
    applications,
  );
  return (
    <Box className="absolute inset-0">
      <PressArea
        testID="notification-center-outside"
        accessible={false}
        className="absolute inset-0"
        onPress={close}
      />
      <Box
        testID="notification-center"
        accessibilityRole="list"
        accessibilityLabel="Centro de notificaciones"
        className="absolute rounded-[22px] border border-white/60"
        style={{
          top: top + 8,
          right: 12,
          bottom: 12,
          width: BANNER_WIDTH + 24,
          backgroundColor: "#e9edf4d9",
          boxShadow: "0 22px 70px #0b183a40, 0 0 0 .5px #0b183a1f",
        }}
      >
        <Box className="flex-row items-center justify-between px-4 pb-2 pt-3">
          <Text accessibilityRole="header" className="text-[15px] font-semibold text-[#1d2530]">
            Notificaciones
          </Text>
          {notifications.doNotDisturb ? (
            <Text testID="notification-center-dnd" className="text-[12px] text-[#5b6573]">
              No molestar
            </Text>
          ) : null}
        </Box>
        {groups.length === 0 ? (
          <Box className="flex-1 items-center justify-center">
            <Text className="text-[13px] text-[#5b6573]">No hay notificaciones</Text>
          </Box>
        ) : (
          <Scroll className="flex-1" contentContainerClassName="gap-4 px-3 pb-3">
            {groups.map((group) => (
              <Box key={group.appId} className="gap-2">
                <Box className="flex-row items-center justify-between px-1">
                  <Box className="flex-row items-center gap-2">
                    {group.app ? <AppIcon app={group.app} size={18} /> : null}
                    <Text accessibilityRole="header" className="text-[12px] font-semibold text-[#3a4452]">
                      {group.name}
                    </Text>
                  </Box>
                  <Button
                    testID={`notification-clear-${group.appId}`}
                    size="sm"
                    appearance="plain"
                    accessibilityLabel={`Borrar las notificaciones de ${group.name}`}
                    onPress={() => group.notifications.forEach((notification) => notifications.remove(notification.id))}
                  >
                    Borrar
                  </Button>
                </Box>
                {group.notifications.map((notification) => (
                  <NotificationCard
                    key={notification.id}
                    testID={`notification-${notification.id}`}
                    notification={notification}
                    applications={applications}
                    now={now}
                    closeLabel="Borrar"
                    close={() => notifications.remove(notification.id)}
                    activate={(action, reply) => {
                      notifications.activate(notification.id, action, reply);
                      close();
                    }}
                  />
                ))}
              </Box>
            ))}
          </Scroll>
        )}
      </Box>
    </Box>
  );
}
