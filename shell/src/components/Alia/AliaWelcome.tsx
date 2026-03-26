import { motion } from "framer-motion";
import AliaFace from "./AliaFace";

interface Suggestion {
  id: string;
  title: string;
  description: string;
}

interface AliaWelcomeProps {
  onSuggestionPress: (text: string) => void;
}

const SUGGESTIONS: Suggestion[] = [
  { id: "1", title: "Write a story", description: "Write a short sci-fi story about an AI" },
  { id: "2", title: "Explain code", description: "How does a binary search algorithm work?" },
  { id: "3", title: "Draft an email", description: "Write a professional follow-up email" },
  { id: "4", title: "Brainstorm", description: "Give me ideas for a weekend project" },
];

function getGreeting(): string {
  const hour = new Date().getHours();
  if (hour >= 5 && hour < 12) return "Good morning";
  if (hour >= 12 && hour < 17) return "Good afternoon";
  return "Good evening";
}

const GREETING = getGreeting();

export default function AliaWelcome({ onSuggestionPress }: AliaWelcomeProps) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center px-4">
      <div className="w-full max-w-2xl">
        {/* Face + Title */}
        <div className="mb-8 flex flex-col items-start">
          <div className="mb-4">
            <AliaFace size={56} expression="Greeting" />
          </div>
          <div className="flex flex-col gap-1">
            <h1 className="text-[26px] font-bold tracking-tight text-white/90">
              {GREETING}
            </h1>
            <p className="text-[16px] font-medium text-white/45">
              How can I help you today?
            </p>
          </div>
        </div>

        {/* Suggestion Grid */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.4 }}
          className="flex flex-wrap gap-2"
        >
          {SUGGESTIONS.map((item) => (
            <button
              key={item.id}
              onClick={() => onSuggestionPress(item.description)}
              className="flex min-w-[35%] flex-1 cursor-pointer flex-col items-start overflow-hidden rounded-3xl border border-white/12 bg-white/6 p-4 text-left backdrop-blur-sm transition-colors hover:bg-white/10"
            >
              <span className="mb-1 text-[13px] font-medium text-white/80">
                {item.title}
              </span>
              <span className="line-clamp-1 text-[12px] text-white/40">
                {item.description}
              </span>
            </button>
          ))}
        </motion.div>
      </div>
    </div>
  );
}
