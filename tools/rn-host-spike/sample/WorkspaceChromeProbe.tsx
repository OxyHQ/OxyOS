import { AppShell } from "@oxy.so/bloom/app-shell";
import { Button } from "@oxy.so/bloom/button";
import { BloomProvider } from "@oxy.so/bloom/provider";
import { Screen, ScreenScrollView } from "@oxy.so/bloom/screen";
import { TabBar, TabBarButton } from "@oxy.so/bloom/tab-bar";
import { Text } from "@oxy.so/bloom/typography";
import * as React from "react";
import { View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { oxyosSurfaces } from "./oxyos/platform-surface";

type Rect = { x: number; y: number; width: number; height: number };
type MeasuredView = React.ComponentRef<typeof View>;
function rectangle(
	ref: React.RefObject<MeasuredView | null>,
): Promise<Rect | null> {
	return new Promise((resolve) => {
		if (!ref.current) return resolve(null);
		ref.current.measureInWindow((x, y, width, height) =>
			resolve({ x, y, width, height }),
		);
	});
}

function Workspace() {
	const header = React.useRef<MeasuredView>(null);
	const panel = React.useRef<MeasuredView>(null);
	const firstRow = React.useRef<MeasuredView>(null);
	const bar = React.useRef<MeasuredView>(null);
	const phase = React.useRef(0);
	const observedScroll = React.useRef(0);
	const [status, setStatus] = React.useState("Workspace not measured");
	const [selected, setSelected] = React.useState(0);
	const measure = async () => {
		const measuredPhase = phase.current;
		const [headerRect, panelRect, firstRect, barRect] = await Promise.all([
			rectangle(header),
			rectangle(panel),
			rectangle(firstRow),
			rectangle(bar),
		]);
		if (
			phase.current !== measuredPhase ||
			!headerRect ||
			!panelRect ||
			!firstRect ||
			!barRect
		)
			return;
		const rounded = (n: number) => Math.round(n);
		const panelUnderHeader = Math.abs(panelRect.y - headerRect.y) <= 1;
		const contentClearance = firstRect.y - panelRect.y + observedScroll.current;
		const clearsHeader = Math.abs(contentClearance - headerRect.height) <= 1;
		const measuredBar = barRect.height >= 70 && barRect.height <= 90;
		setStatus(
			`Panel under header ${panelUnderHeader ? "yes" : "no"}; Content clears header ${clearsHeader ? "yes" : "no"}; Bar measured ${measuredBar ? "yes" : "no"}\n` +
				`H${rounded(headerRect.y)}/${rounded(headerRect.height)} P${rounded(panelRect.y)}/${rounded(panelRect.height)} C${rounded(firstRect.y)} B${rounded(barRect.y)}/${rounded(barRect.height)} S${observedScroll.current}`,
		);
	};
	return (
		<AppShell
			header={null}
			scroll="page"
			framedFrom={1000}
			gutter={0}
			contentMaxWidth="none"
			reserveBottomBarSpace={false}
			bottomBarVisibility="always"
			bottomBar={
				<View ref={bar} collapsable={false}>
					<TabBar activeIndex={selected} onIndexChange={setSelected}>
						<TabBarButton
							index={0}
							item={{ name: "home", label: "Home", icon: null }}
						/>
						<TabBarButton
							index={1}
							item={{ name: "settings", label: "Settings", icon: null }}
						/>
					</TabBar>
				</View>
			}
			panelLayout={{
				flow: "document",
				headerPlacement: "scroll-under",
				header: (
					<View
						ref={header}
						collapsable={false}
						style={{ height: 88, paddingLeft: 96 }}
					>
						<View style={{ flexDirection: "row", gap: 8 }}>
							<Button size="sm" onPress={measure}>
								Measure workspace
							</Button>
						</View>
						<Text style={{ fontSize: 11, lineHeight: 15 }}>{status}</Text>
					</View>
				),
				panels: [
					{
						id: "main",
						content: (
							<View ref={panel} collapsable={false} style={{ flex: 1 }}>
								<Screen scroll="content" documentScroll contentClearance={0}>
									<ScreenScrollView
										testID="workspace-scroll"
										onScroll={(event) => {
											phase.current += 1;
											observedScroll.current =
												event.nativeEvent.contentOffset.y;
											setStatus("Workspace not measured");
										}}
									>
										<View
											ref={firstRow}
											collapsable={false}
											style={{ height: 48 }}
										>
											<Text>First content row</Text>
										</View>
										{Array.from({ length: 24 }, (_, index) => (
											<Text key={index} style={{ height: 48 }}>
												Workspace row {index + 1}
											</Text>
										))}
									</ScreenScrollView>
								</Screen>
							</View>
						),
					},
				],
			}}
		/>
	);
}
export default function WorkspaceChromeProbe() {
	return (
		<SafeAreaProvider>
			<BloomProvider
				platform={oxyosSurfaces}
				colorPreset="sky"
				mode="light"
				fonts={false}
			>
				<Workspace />
			</BloomProvider>
		</SafeAreaProvider>
	);
}
