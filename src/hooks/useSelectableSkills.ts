import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import { listSkills, type SkillMetadata } from "../api";

export function useSelectableSkills() {
  const [availableSkills, setAvailableSkills] = useState<SkillMetadata[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);

  useEffect(() => {
    let disposed = false;

    const load = async () => {
      try {
        const skills = await listSkills();
        if (disposed) return;
        setAvailableSkills(
          skills
            .sort((left, right) =>
              `${left.source}:${left.name}`.localeCompare(`${right.source}:${right.name}`),
            ),
        );
      } catch {
        if (!disposed) setAvailableSkills([]);
      }
    };

    void load();
    const unlistenPromise = listen("skills_updated", () => {
      void load();
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    setSelectedSkillIds((current) =>
      current.filter((id) => availableSkills.some((skill) => skill.id === id)),
    );
  }, [availableSkills]);

  const selectedSkills = useMemo(
    () =>
      selectedSkillIds
        .map((id) => availableSkills.find((skill) => skill.id === id) ?? null)
        .filter((skill): skill is SkillMetadata => Boolean(skill)),
    [availableSkills, selectedSkillIds],
  );

  const toggleSkillSelection = (skillId: string) => {
    setSelectedSkillIds((current) =>
      current.includes(skillId)
        ? current.filter((id) => id !== skillId)
        : [...current, skillId],
    );
  };

  const clearSelectedSkills = () => {
    setSelectedSkillIds([]);
  };

  return {
    availableSkills,
    selectedSkillIds,
    selectedSkills,
    setSelectedSkillIds,
    toggleSkillSelection,
    clearSelectedSkills,
  };
}
