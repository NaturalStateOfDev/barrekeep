import { useState } from "react";
import { CalendarDays, Users, Shapes, Settings } from "lucide-react";
import { UpdateBanner } from "./components/UpdateBanner";
import { ProposalsScreen } from "./screens/ProposalsScreen";
import { TeachersScreen } from "./screens/TeachersScreen";
import { PositionsScreen } from "./screens/PositionsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";

type View = "proposals" | "teachers" | "positions" | "settings";

const NAV: Array<[View, typeof CalendarDays, string]> = [
  ["proposals", CalendarDays, "Proposals"],
  ["teachers", Users, "Teachers"],
  ["positions", Shapes, "Class types"],
];

export function App() {
  const [view, setView] = useState<View>("proposals");
  const goSettings = () => setView("settings");

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="wordmark">
          barre<em>keep</em>
        </div>
        {NAV.map(([id, Icon, label]) => (
          <button
            key={id}
            className={`nav-item ${view === id ? "active" : ""}`}
            onClick={() => setView(id)}
          >
            <Icon size={18} /> {label}
          </button>
        ))}
        <button
          className={`nav-item ${view === "settings" ? "active" : ""}`}
          onClick={goSettings}
          style={{ marginTop: "auto" }}
        >
          <Settings size={18} /> Settings
        </button>
      </aside>
      <main className="main">
        <UpdateBanner />
        {view === "proposals" && <ProposalsScreen onGoSettings={goSettings} />}
        {view === "teachers" && <TeachersScreen onGoSettings={goSettings} />}
        {view === "positions" && <PositionsScreen />}
        {view === "settings" && <SettingsScreen />}
      </main>
    </div>
  );
}
