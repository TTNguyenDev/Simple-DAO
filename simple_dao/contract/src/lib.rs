use near_sdk::{
    AccountId, 
    Balance,
    env,
    near_bindgen,
    Promise,
    Duration
};

use near_sdk::borsh::{
    self,
    BorshDeserialize,
    BorshSerialize
};

use near_sdk::collections::{
    UnorderedSet,
    UnorderedMap,
    Vector
};

use near_sdk::serde::{
    Serialize,
    Deserialize
};

use std::collections::HashMap;
use near_sdk::json_types::{
    WrappedBalance,
    WrappedDuration
};

const CONSENSUS_PERCENTAGE: u128 = 50; 

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[serde(crate="near_sdk::serde")]
pub enum Vote {
    Yes, 
    No
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, PartialEq, Debug, Clone)]
#[cfg_attr(not(target_arch="wasm32"), derive(Eq))]
#[serde(crate="near_sdk::serde")]
pub enum ProposalStatus {
    Vote, 
    Success,
    Fail
}

impl ProposalStatus {
    pub fn is_finalized(&self) -> bool {
        self != &ProposalStatus::Vote
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[serde(crate="near_sdk::serde")]
#[serde(tag="type")]
pub enum ProposalType {
    NewCouncil { amount: WrappedBalance },
    DeleteCouncil,
    Payout { amount: WrappedBalance },
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[serde(crate="near_sdk::serde")]
pub struct Council {
    account: AccountId,
    weight: u128,
    locked_tokens: Balance 
}

#[near_bindgen]
#[derive(BorshSerialize, BorshDeserialize)]
pub struct Proposal {
    status: ProposalStatus,
    proposer: AccountId,
    receiver: AccountId,
    description: String,
    kind: ProposalType,
    vote_period_end: Duration,
    votes: UnorderedMap<Council, Vote>
}

impl Proposal {
    pub fn get_amount(&self) -> Option<Balance> {
        match self.kind {
            ProposalType::Payout {amount} => Some(amount.0),
            _ => None,
        }
    }

    pub fn vote_status(&self) -> ProposalStatus {
        let zero: u128 = 0;

        let percent: u128 = self.votes
            .iter()
            .map(|(k, v)| {
                match v {
                    Vote::Yes => k.weight,
                    Vote::No => zero
                }
            })
        .sum();

        if percent >= CONSENSUS_PERCENTAGE {
            ProposalStatus::Success
        } else if percent < CONSENSUS_PERCENTAGE && env::block_timestamp() > self.vote_period_end {
            ProposalStatus::Fail
        } else {
            ProposalStatus::Vote
        } 
    }
}

#[derive(Serialize, Deserialize)]
#[serde(crate="near_sdk::serde")]
pub struct ProposalInput {
    target: AccountId,
    description: String,
    kind: ProposalType
}

#[near_bindgen]
#[derive(BorshSerialize, BorshDeserialize)]
pub struct DAO {
    purpose: String,
    bond: Balance,
    vote_period: Duration,
    grace_period: Duration,
    council: UnorderedSet<Council>,
    proposals: Vector<Proposal>
}

impl Default for DAO {
    fn default() -> Self {
        env::panic(b"DAO should be initialized before usage")
    } 
}

#[near_bindgen]
impl DAO {
    #[init] 
    #[payable]
    pub fn new(
        _purpose: String,
        _bond: WrappedBalance,
        _vote_period: WrappedDuration,
        _grace_period: WrappedDuration
        ) -> Self {
        assert!(!env::state_exists(), "DAO contract is already initialized");

        let mut dao = Self {
            purpose: _purpose,
            bond: _bond.into(),
            vote_period: _vote_period.into(),
            grace_period: _grace_period.into(),
            council: UnorderedSet::new(b"c".to_vec()),
            proposals: Vector::new(b"p".to_vec()),
        };

        let zero: u128 = 0;
        let owner = Council {
            account: env::predecessor_account_id(),
            weight: zero,
            locked_tokens: env::attached_deposit()
        };

        dao.council.insert(&owner);
        dao
    }

    #[payable]
    pub fn add_proposal(
        &mut self,
        _proposal: ProposalInput
        ) -> u64 {
        assert!(env::attached_deposit() >= self.bond, "Not enough deposit");

        let p = Proposal {
            status: ProposalStatus::Vote,
            proposer: env::predecessor_account_id(),
            receiver: _proposal.target,
            description: _proposal.description,
            kind: _proposal.kind,
            vote_period_end: env::block_timestamp() + self.vote_period,
            votes: UnorderedMap::new(b"v".to_vec())
        };

        self.proposals.push(&p);
        self.proposals.len() - 1
    }

    pub fn get_proposals_by_status(
        &self,
        _status: ProposalStatus
        ) -> HashMap<u64, Proposal> {
        let ids: Vec<u64> = (0..self.proposals.len())
            .filter(|index| self.proposals.get(index.clone()).unwrap().status == _status) 
            .collect();

        (0..ids.len())
            .map(|index| {
                let id: u64 = ids[index as usize];
                (id, self.proposals.get(id).unwrap())
            })
        .collect()
    }

    pub fn vote(
        &mut self, 
        id: u64, 
        vote: Vote
        ) {
        let councils: Vec<Council> = self.council
            .iter()
            .filter(|item| item.account == env::predecessor_account_id())
            .collect();

        assert!(
            councils.len() > 0,
            "Only council can vote"
        );
        
        let mut proposal = self.proposals.get(id).expect("No proposal with such id");
        assert_eq!(
            proposal.status,
            ProposalStatus::Vote,
            "Proposal already finalized"
        );

        if proposal.vote_period_end < env::block_timestamp() {
            env::log(b"Voting period expried, finalizing the proposal");
            self.finalized(id);
            return;
        }

        let already_votes: Vec<Council> = proposal.votes
            .keys()
            .filter(|k| k.account == env::predecessor_account_id())
            .collect();

        assert!(
            already_votes.len() == 0,
            "Already voted"
        );

        let council = councils.get(0).clone().unwrap();
        
        proposal.votes.insert(council, &vote);
        let post_status = proposal.vote_status();

        //Update status after voting
        proposal.status = post_status.clone();
        self.proposals.replace(id, &proposal);

        if post_status.is_finalized() {
            self.finalized(id);
        }
    }

    pub fn finalized(
        &mut self, 
        id: u64
        ) {
        let mut proposal = self.proposals.get(id).expect("No proposal with such id");

        assert!(
            !proposal.status.is_finalized(),
            "Proposal already finalized"
        );

        proposal.status = proposal.vote_status();
        match proposal.status {
            ProposalStatus::Success => {
                env::log(b"Vote succeded");

                let target = proposal.receiver.clone();
                //Send bond back to proposer
                Promise::new(proposal.proposer.clone()).transfer(self.bond);

                match proposal.kind {
                    ProposalType::NewCouncil { amount } => {
                        let council = Council {
                            account: target,
                            weight: 10, //HARD CODE
                            locked_tokens: amount.0 
                        };

                        self.council.insert(&council);
                        self.recompute_percentage();
                    }

                    ProposalType::DeleteCouncil => {
                        let councils: Vec<Council> = self.council
                            .iter()
                            .filter(|item| item.account == target)
                            .collect();

                        let council = councils.get(0).clone().unwrap();
                        let locked_tokens = council.locked_tokens;

                        Promise::new(council.account.clone()).transfer(locked_tokens);
                        
                        self.council.remove(&council);
                        self.recompute_percentage();
                    } 

                    ProposalType::Payout { amount } => {
                        Promise::new(target).transfer(amount.0);
                    }
                }
            }

            ProposalStatus::Fail => {
                env::log(b"Proposal vote failed");
                //Send bond back to proposer
                Promise::new(proposal.proposer.clone()).transfer(self.bond); 
            }

            ProposalStatus::Vote => {
                env::panic(b"Voting period has not expired and no majority vote yet");
            }
        }
    }

    fn recompute_percentage(
        &mut self
        ) {
        let sum: Balance = self.council
            .iter()
            .map(|item| item.locked_tokens)
            .sum();

        for mut item in self.council.iter() {
            item.weight = item.locked_tokens / sum;
        }
    }
}
