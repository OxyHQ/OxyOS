import React, { useEffect, useRef, useState } from "react";
import { useWindowDimensions } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { BloomColorScope } from "@oxy.so/bloom/theme";
import { Search } from "@oxy.so/bloom/search";
import { Box, PressArea } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication, ShellFolder } from "../model";

const normalize = (value: string) =>
  value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase();
type Entry = { app: ShellApplication } | { folder: ShellFolder };

export function Launchpad({
  launch,
  dismiss,
  applications,
  folders,
  escapeRequest = 0,
}: {
  launch: (app: string) => void;
  applications: readonly ShellApplication[];
  folders: readonly ShellFolder[];
  dismiss: () => void;
  escapeRequest?: number;
}) {
  const [query, setQuery] = useState("");
  const [folder, setFolder] = useState<ShellFolder | null>(null);
  const [page, setPage] = useState(0);
  const { width, height } = useWindowDimensions();
  const close = useRef(() => {});
  close.current = () => {
    if (folder) setFolder(null);
    else if (query) setQuery("");
    else dismiss();
  };
  const lastEscape = useRef(escapeRequest);
  useEffect(() => {
    if (lastEscape.current !== escapeRequest) close.current();
    lastEscape.current = escapeRequest;
  }, [escapeRequest]);
  const grouped = new Set(folders.flatMap((group) => [...group.appIds]));
  const entries: Entry[] = applications
    .filter(
      (app) =>
        (query || !grouped.has(app.id)) &&
        normalize(app.name).includes(normalize(query)),
    )
    .map((app) => ({ app }));
  if (!query) entries.push(...folders.map((group) => ({ folder: group })));
  const columns = width >= 700 ? 7 : 4;
  const rows = height >= 600 ? 5 : 3;
  const capacity = columns * rows;
  const pageCount = Math.max(1, Math.ceil(entries.length / capacity));
  const currentPage = Math.min(page, pageCount - 1);
  const gridWidth = Math.min(1120, width * 0.8);
  // Preserve Launchpad's seven-column rhythm, fitting five rows between the search and shelf.
  const rowHeight = Math.min(126, (height - 180) / rows);
  const iconSize = Math.max(
    40,
    Math.min(68, (gridWidth / columns) * 0.48, rowHeight - 35),
  );
  const folderWidth = Math.min(640, width - 64);
  const folderColumns = width >= 700 ? 5 : 3;
  const folderApps = applications.filter((app) =>
    folder?.appIds.includes(app.id),
  );
  const labelStyle = {
    textShadowColor: "#10204a80",
    textShadowOffset: { width: 0, height: 1 },
    textShadowRadius: 3,
  };
  return (
    <Box
      testID="shell-launchpad"
      className="absolute inset-0 items-center bg-[#101760]/45"
      style={{ zIndex: 20 }}
    >
      <PressArea
        accessibilityRole="button"
        accessibilityLabel="Cerrar aplicaciones"
        className="absolute inset-0 h-full w-full rounded-none border-0 bg-transparent p-0"
        onPress={dismiss}
      />
      <Box className="mb-5 mt-5 w-[220px]">
        <BloomColorScope
          seed="#4464ba"
          mode="dark"
          tokens={{
            background: "#294fa1",
            surface: "#4769b6",
            foreground: "#ffffff",
            primary: "#c5d8ff",
            border: "#ffffff33",
          }}
        >
          <Search
            label="Buscar aplicaciones"
            placeholder="Buscar"
            accessibilityLabel="Buscar aplicaciones"
            iconSize={14}
            value={query}
            onChangeText={(value) => {
              setQuery(value);
              setPage(0);
            }}
            onClearText={() => {
              setQuery("");
              setPage(0);
            }}
            fieldClassName="h-7 min-h-0"
            fieldChromeClassName="border-white/20 bg-white/10"
            clearButtonProps={{ size: 20, glyphSize: 12 }}
            onKeyPress={(event) => {
              if (event.nativeEvent.key === "Escape") {
                event.stopPropagation();
                close.current();
              }
            }}
            onSubmitEditing={() => {
              const first = entries[0];
              if (first && "app" in first) launch(first.app.id);
            }}
            style={{
              color: "#fff",
              fontSize: 12,
              height: 28,
              minHeight: 28,
              paddingTop: 0,
              paddingBottom: 0,
            }}
          />
        </BloomColorScope>
      </Box>
      <Box className="flex-row flex-wrap" style={{ width: gridWidth }}>
        {entries
          .slice(currentPage * capacity, (currentPage + 1) * capacity)
          .map((entry) => {
            const app = "app" in entry ? entry.app : undefined;
            const group = "folder" in entry ? entry.folder : undefined;
            return (
              <Button
                key={app?.id ?? group!.id}
                testID={
                  app ? `launcher-${app.id}` : `launcher-folder-${group!.id}`
                }
                accessibilityLabel={
                  app
                    ? `Abrir ${app.name} desde aplicaciones`
                    : `Abrir carpeta ${group!.name}`
                }
                appearance="plain"
                className="flex-col items-center justify-start rounded-xl border-0 bg-transparent p-0"
                style={{ width: gridWidth / columns, height: rowHeight }}
                onPress={() => (app ? launch(app.id) : setFolder(group!))}
              >
                <Box className="items-center gap-2 pt-1">
                  {app ? (
                    <AppIcon app={app} size={iconSize} />
                  ) : (
                    <Box
                      className="flex-row flex-wrap content-start border border-white/20 bg-[#dce2f4]/50"
                      style={{
                        width: iconSize,
                        height: iconSize,
                        borderRadius: iconSize * 0.22,
                        padding: iconSize * 0.11,
                        gap: iconSize * 0.055,
                      }}
                    >
                      {applications
                        .filter((item) => group!.appIds.includes(item.id))
                        .slice(0, 9)
                        .map((item) => (
                          <AppIcon
                            key={item.id}
                            app={item}
                            size={iconSize * 0.21}
                          />
                        ))}
                    </Box>
                  )}
                  <Text
                    numberOfLines={1}
                    className="text-[12px] font-normal text-white"
                    style={labelStyle}
                  >
                    {app?.name ?? group!.name}
                  </Text>
                </Box>
              </Button>
            );
          })}
      </Box>
      {!entries.length && (
        <Text className="mt-12 text-center text-sm text-white/80">
          No hay aplicaciones que coincidan con «{query}».
        </Text>
      )}
      <Box className="absolute bottom-5 flex-row items-center justify-center gap-1">
        {Array.from({ length: pageCount }, (_, index) => (
          <Button
            key={index}
            accessibilityLabel={`Página ${index + 1}`}
            pressed={index === currentPage}
            appearance="plain"
            className="h-6 w-5 rounded-full border-0 bg-transparent p-0"
            onPress={() => setPage(index)}
          >
            <Box
              className="h-[5px] w-[5px] rounded-full"
              style={{
                backgroundColor: index === currentPage ? "#fff" : "#ffffff50",
              }}
            />
          </Button>
        ))}
      </Box>
      {folder && (
        <Box
          testID="launcher-folder"
          className="absolute inset-0 items-center justify-center bg-[#132447]/20"
        >
          <PressArea
            accessibilityRole="button"
            accessibilityLabel={`Cerrar carpeta ${folder.name}`}
            className="absolute inset-0 h-full w-full rounded-none border-0 bg-transparent p-0"
            onPress={() => setFolder(null)}
          />
          <Box style={{ width: folderWidth }}>
            <Text
              className="mb-5 text-center text-[24px] font-normal text-white"
              style={labelStyle}
            >
              {folder.name}
            </Text>
            <Box
              className="flex-row flex-wrap rounded-[28px] border border-white/30 bg-[#dce1ef]/75 p-6"
              style={{
                boxShadow: "0 18px 60px #091a4b30, inset 0 1px 0 #ffffff40",
              }}
            >
              {folderApps.map((app) => (
                <Button
                  key={app.id}
                  accessibilityLabel={`Abrir ${app.name} desde ${folder.name}`}
                  appearance="plain"
                  className="h-[108px] items-center justify-center rounded-xl border-0 bg-transparent p-0"
                  style={{ width: (folderWidth - 50) / folderColumns }}
                  onPress={() => launch(app.id)}
                >
                  <Box className="items-center gap-2">
                    <AppIcon app={app} size={Math.min(62, iconSize)} />
                    <Text
                      numberOfLines={1}
                      className="text-[12px] font-normal text-[#35405a]"
                    >
                      {app.name}
                    </Text>
                  </Box>
                </Button>
              ))}
            </Box>
          </Box>
        </Box>
      )}
    </Box>
  );
}
