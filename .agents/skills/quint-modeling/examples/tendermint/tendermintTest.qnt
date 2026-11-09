module valid {

  import basicSpells.* from "./spells/basicSpells"

  import tendermint(
    F = 1,
    CORRECT = Set("p1","p2", "p3"),
    FAULTY = Set("p4"),
    PROPOSER = Map(0 -> "p1", 1 -> "p2", 2 -> "p3", 3 -> "p4"),
    VALUES =  Map(0 -> "v0", 1 -> "v1", 2 -> "v2", 3 -> "v3"),
    MAX_ROUND = 4
  ).* from "./tendermint"

  run line28Test = {
    val v0_proposal = { proposal: "v0", round: 0, src: "p1", valid_round: -1 }

    init
      .then("p1".with_cue(listen_proposal_in_propose, v0_proposal).perform(broadcast_prevote_for_proposal))
      .then("p2".with_cue(listen_proposal_in_propose, v0_proposal).perform(broadcast_prevote_for_proposal))

      .then("p1".with_cue(listen_proposal_in_prevote_commit, v0_proposal).perform(lock_value_and_precommit))
      .then("p2".with_cue(listen_proposal_in_prevote_commit, v0_proposal).perform(lock_value_and_precommit))

      .then("p2".with_cue(listen_quorum_precommits_any, ()).perform(trigger_precommit_timeout))

      .then(step_with("p2", on_precommit_timeout))
      .expect(s.system.get("p2").round == 1)

      .expect(and {
        s.system.get("p2").stage == ProposeStage,
        s.messages.values().forall(ms => {
          ms.contains(Propose({ src: "p2", round: 1, proposal: "v0", valid_round: 0 }))
        })
      })

      .then("p2"
        .with_cue(listen_proposal_in_propose_prevote, { proposal: "v0", round: 1, src: "p2", valid_round: 0 })
        .perform(broadcast_prevote_with_validation)
      )
  }
}

module no_agreement {

  import tendermint(
    F = 1,
    CORRECT = Set("p1", "p2"),
    FAULTY = Set("p3","p4"),
    PROPOSER = Map(0 -> "p3", 1 -> "p2", 2 -> "p3", 3 -> "p4"),
    VALUES =  Map(0 -> "v0", 1 -> "v1", 2 -> "v2", 3 -> "v3"),
    MAX_ROUND = 4
  ).* from "./tendermint"

  run disagreementTest = {
    val v0_proposal = { proposal: "v0", round: 0, src: "p3", valid_round: -1 }
    val v1_proposal = { proposal: "v1", round: 0, src: "p3", valid_round: -1 }

    init
      .then("p1".with_cue(listen_proposal_in_propose, v0_proposal).perform(broadcast_prevote_for_proposal))
      .then("p2".with_cue(listen_proposal_in_propose, v1_proposal).perform(broadcast_prevote_for_proposal))
      .then("p1".with_cue(listen_proposal_in_prevote_commit, v0_proposal).perform(lock_value_and_precommit))
      .then("p2".with_cue(listen_proposal_in_prevote_commit, v1_proposal).perform(lock_value_and_precommit))
      .then("p1".with_cue(listen_proposal_in_precommit_no_decision, v0_proposal).perform(decide_on_proposal))
      .then("p2".with_cue(listen_proposal_in_precommit_no_decision, v1_proposal).perform(decide_on_proposal))
      .expect(not(agreement))
  }
}
