use serde::{Deserialize, Serialize};

/// User-visible generation modes. `SequentialCrew` is the preferred automatic
/// engine for expensive multi-actor scenes; `BigScene` remains an explicit
/// compatibility mode for the former parallel crew implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationMode {
    Continuation,
    Duet,
    BigScene,
    SequentialCrew,
}

impl GenerationMode {
    pub fn estimated_call_label(self) -> &'static str {
        match self {
            Self::Continuation => "1 次正文 + 2 次廉价记账",
            Self::Duet => "3–5 次正文编排 + 2 次廉价记账",
            Self::BigScene => "4+N 次正文编排 + 2 次廉价记账",
            Self::SequentialCrew => "2+N 次顺序编排 + 2 次廉价记账",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationRouteReason {
    ExplicitChoice,
    ActorCount,
    LargeSceneIntent,
    MultipleImminentTasks,
    DuetSignals,
    EconomyDefault,
}

/// Deterministic, auditable inputs for mode routing. Callers must only set
/// `relevant_private_knowledge_divergence` when the differing knowledge is
/// relevant to this turn; an unrelated secret must not upgrade every scene.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationRouteSignals {
    pub explicit_mode: Option<GenerationMode>,
    pub principal_actor_count: usize,
    pub large_scene_intent: bool,
    pub imminent_task_count: usize,
    pub direct_interaction: bool,
    pub relevant_private_knowledge_divergence: bool,
    pub opposing_agendas: bool,
    pub high_tension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationRouteDecision {
    pub mode: GenerationMode,
    pub reason: GenerationRouteReason,
    /// Automatic upgrades into the expensive big-scene tier require an
    /// explicit confirmation. An explicit user choice never asks twice.
    pub requires_cost_confirmation: bool,
}

pub fn route_generation_mode(signals: &GenerationRouteSignals) -> GenerationRouteDecision {
    if let Some(mode) = signals.explicit_mode {
        return GenerationRouteDecision {
            mode,
            reason: GenerationRouteReason::ExplicitChoice,
            requires_cost_confirmation: false,
        };
    }

    let big_scene_reason = if signals.principal_actor_count >= 4 {
        Some(GenerationRouteReason::ActorCount)
    } else if signals.principal_actor_count >= 3 && signals.large_scene_intent {
        Some(GenerationRouteReason::LargeSceneIntent)
    } else if signals.principal_actor_count >= 3 && signals.imminent_task_count >= 2 {
        Some(GenerationRouteReason::MultipleImminentTasks)
    } else {
        None
    };
    if let Some(reason) = big_scene_reason {
        return GenerationRouteDecision {
            mode: GenerationMode::SequentialCrew,
            reason,
            requires_cost_confirmation: true,
        };
    }

    if signals.principal_actor_count == 2 {
        let duet_score = usize::from(signals.direct_interaction) * 2
            + usize::from(signals.relevant_private_knowledge_divergence) * 2
            + usize::from(signals.opposing_agendas)
            + usize::from(signals.high_tension);
        if duet_score >= 2 {
            return GenerationRouteDecision {
                mode: GenerationMode::Duet,
                reason: GenerationRouteReason::DuetSignals,
                requires_cost_confirmation: false,
            };
        }
    }

    GenerationRouteDecision {
        mode: GenerationMode::Continuation,
        reason: GenerationRouteReason::EconomyDefault,
        requires_cost_confirmation: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(actor_count: usize) -> GenerationRouteSignals {
        GenerationRouteSignals {
            principal_actor_count: actor_count,
            ..GenerationRouteSignals::default()
        }
    }

    #[test]
    fn explicit_mode_always_wins_without_surprise_confirmation() {
        let mut input = signals(6);
        input.explicit_mode = Some(GenerationMode::Continuation);
        input.large_scene_intent = true;

        let decision = route_generation_mode(&input);

        assert_eq!(decision.mode, GenerationMode::Continuation);
        assert_eq!(decision.reason, GenerationRouteReason::ExplicitChoice);
        assert!(!decision.requires_cost_confirmation);
    }

    #[test]
    fn four_or_more_principal_actors_route_to_sequential_crew() {
        let decision = route_generation_mode(&signals(4));

        assert_eq!(decision.mode, GenerationMode::SequentialCrew);
        assert_eq!(decision.reason, GenerationRouteReason::ActorCount);
        assert!(decision.requires_cost_confirmation);
    }

    #[test]
    fn three_actors_and_large_scene_intent_route_to_sequential_crew() {
        let mut input = signals(3);
        input.large_scene_intent = true;

        let decision = route_generation_mode(&input);

        assert_eq!(decision.mode, GenerationMode::SequentialCrew);
        assert_eq!(decision.reason, GenerationRouteReason::LargeSceneIntent);
    }

    #[test]
    fn multiple_imminent_tasks_raise_three_actor_turn_to_sequential_crew() {
        let mut input = signals(3);
        input.imminent_task_count = 2;

        let decision = route_generation_mode(&input);

        assert_eq!(decision.mode, GenerationMode::SequentialCrew);
        assert_eq!(
            decision.reason,
            GenerationRouteReason::MultipleImminentTasks
        );
    }

    #[test]
    fn two_actor_direct_interaction_routes_to_duet() {
        let mut input = signals(2);
        input.direct_interaction = true;

        let decision = route_generation_mode(&input);

        assert_eq!(decision.mode, GenerationMode::Duet);
        assert_eq!(decision.reason, GenerationRouteReason::DuetSignals);
        assert!(!decision.requires_cost_confirmation);
    }

    #[test]
    fn relevant_private_knowledge_divergence_routes_to_duet() {
        let mut input = signals(2);
        input.relevant_private_knowledge_divergence = true;

        assert_eq!(route_generation_mode(&input).mode, GenerationMode::Duet);
    }

    #[test]
    fn two_weak_duet_signals_are_enough_but_one_is_not() {
        let mut input = signals(2);
        input.opposing_agendas = true;
        assert_eq!(
            route_generation_mode(&input).mode,
            GenerationMode::Continuation
        );

        input.high_tension = true;
        assert_eq!(route_generation_mode(&input).mode, GenerationMode::Duet);
    }

    #[test]
    fn ambiguous_turn_defaults_to_the_cheapest_mode() {
        let mut input = signals(3);
        input.opposing_agendas = true;

        let decision = route_generation_mode(&input);

        assert_eq!(decision.mode, GenerationMode::Continuation);
        assert_eq!(decision.reason, GenerationRouteReason::EconomyDefault);
        assert!(!decision.requires_cost_confirmation);
    }

    #[test]
    fn generation_mode_uses_stable_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_string(&GenerationMode::Continuation).unwrap(),
            "\"continuation\""
        );
        assert_eq!(
            serde_json::to_string(&GenerationMode::Duet).unwrap(),
            "\"duet\""
        );
        assert_eq!(
            serde_json::to_string(&GenerationMode::BigScene).unwrap(),
            "\"big_scene\""
        );
        assert_eq!(
            serde_json::from_str::<GenerationMode>("\"sequential_crew\"").unwrap(),
            GenerationMode::SequentialCrew,
        );
    }
}
