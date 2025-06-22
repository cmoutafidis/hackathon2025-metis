use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint};
use std::str::FromStr;

declare_id!("TK9uHFJGK2ULY5M7t16EewhaB26KHWs5zmQgBuHyKpS");

pub const JLP_MINT_ADDRESS: &str = "27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4";

#[program]
pub mod solana_yield {
    use super::*;

     pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        supported_chains: Vec<ChainInfo>,
        supported_protocols: Vec<ProtocolInfo>,
    ) -> Result<()> {
        let protocol_registry = &mut ctx.accounts.protocol_registry;
        protocol_registry.admin = *ctx.accounts.admin.key;
        protocol_registry.supported_chains = supported_chains;
        protocol_registry.supported_protocols = supported_protocols;
        protocol_registry.bump = ctx.bumps.protocol_registry;
        Ok(())
    }
 pub fn deposit_funds(
        ctx: Context<DepositFunds>,
        amount: u64,
        risk_tolerance: u8,
        preferred_chains: Option<Vec<u32>>,
    ) -> Result<()> {
        require!(risk_tolerance <= 10, YieldAggregatorError::InvalidRiskTolerance);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::Transfer {
                    from: ctx.accounts.user_token_account.to_account_info(),
                    to: ctx.accounts.vault_token_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;

        let positions = optimize_yield_allocation(
            amount,
            risk_tolerance,
            preferred_chains,
            &ctx.accounts.protocol_registry,
        )?;

        let user_position = &mut ctx.accounts.user_position;
        user_position.owner = *ctx.accounts.user.key;
        user_position.deposited_amount = amount;
        user_position.positions = positions;
        user_position.bump = ctx.bumps.user_position;

        Ok(())
    }

    pub fn withdraw_funds(ctx: Context<WithdrawFunds>, amount: u64) -> Result<()> {
        let user_position = &mut ctx.accounts.user_position;

        require!(
            user_position.deposited_amount >= amount,
            YieldAggregatorError::InsufficientFunds
        );

        let seeds: &[&[u8]] = &[b"vault", &[ctx.accounts.protocol_registry.bump]];
        let signer = &[seeds];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.protocol_registry.to_account_info(),
                },
                signer,
            ),
            amount,
        )?;

        user_position.deposited_amount -= amount;
        Ok(())
    }

    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        let user_position = &mut ctx.accounts.user_position;
        let rewards = calculate_rewards(user_position)?;

        if rewards > 0 {
            user_position.claimed_rewards += rewards;
        }

        Ok(())
    }

    pub fn update_strategies(
        ctx: Context<UpdateStrategies>,
        new_protocols: Vec<ProtocolInfo>,
    ) -> Result<()> {
        require!(
            *ctx.accounts.admin.key == ctx.accounts.protocol_registry.admin,
            YieldAggregatorError::Unauthorized
        );

        ctx.accounts.protocol_registry.supported_protocols = new_protocols;
        Ok(())
    }
}

// Yield optimization logic
fn optimize_yield_allocation(
    amount: u64,
    risk_tolerance: u8,
    preferred_chains: Option<Vec<u32>>,
    protocol_registry: &Account<ProtocolRegistry>,
) -> Result<Vec<Position>> {
    let mut protocols = protocol_registry.supported_protocols.clone();

    protocols.retain(|p| p.risk_score <= risk_tolerance);

    if let Some(chains) = preferred_chains {
        protocols.retain(|p| chains.contains(&p.chain_id));
    }

    require!(!protocols.is_empty(), YieldAggregatorError::NoSuitableProtocols);

    protocols.sort_by(|a, b| {
        let a_score = a.apy * (10 - a.risk_score) as u32;
        let b_score = b.apy * (10 - b.risk_score) as u32;
        b_score.cmp(&a_score)
    });

    let selected = protocols.iter().take(3).collect::<Vec<_>>();
    let base_amount = amount / selected.len() as u64;
    let remainder = amount % selected.len() as u64;

    selected
        .iter()
        .enumerate()
        .map(|(i, protocol)| {
            Ok(Position {
                chain_id: protocol.chain_id,
                protocol_id: protocol.protocol_id,
                amount: base_amount + if i == 0 { remainder } else { 0 },
                start_time: Clock::get()?.unix_timestamp,
                reward_accrued: 0,
            })
        })
        .collect()
}

// Reward calculation
fn calculate_rewards(user_position: &mut Account<UserPosition>) -> Result<u64> {
    let now = Clock::get()?.unix_timestamp;
    let mut total_rewards = 0;

    for position in &mut user_position.positions {
        let days = (now - position.start_time) / 86400;
        let daily_rate = 1500.0 / 36500.0;
        let reward = (position.amount as f64 * daily_rate * days as f64) as u64;

        position.reward_accrued += reward;
        total_rewards += reward;
        position.start_time = now;
    }

    Ok(total_rewards)
}

// Data Structures
#[account]
#[derive(Default)]
pub struct UserPosition {
    pub owner: Pubkey,
    pub deposited_amount: u64,
    pub claimed_rewards: u64,
    pub positions: Vec<Position>,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct Position {
    pub chain_id: u32,
    pub protocol_id: u32,
    pub amount: u64,
    pub start_time: i64,
    pub reward_accrued: u64,
}

#[account]
#[derive(Default)]
pub struct ProtocolRegistry {
    pub admin: Pubkey,
    pub supported_chains: Vec<ChainInfo>,
    pub supported_protocols: Vec<ProtocolInfo>,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct ChainInfo {
    pub chain_id: u32,
    pub bridge_address: String,
    pub gas_token: String,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct ProtocolInfo {
    pub protocol_id: u32,
    pub name: String,
    pub chain_id: u32,
    pub apy: u32,
    pub risk_score: u8,
}

// Errors
#[error_code]
pub enum YieldAggregatorError {
    #[msg("Unauthorized access")]
    Unauthorized,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("Invalid risk tolerance value (0-10)")]
    InvalidRiskTolerance,
    #[msg("No suitable protocols found")]
    NoSuitableProtocols,
}

// Contexts
#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init,
        payer = admin,
        space = 8 + 32 + 4 + (4 + 32 + 4) * 10 + 4 + (4 + 32 + 4 + 4 + 1) * 10 + 1,
        seeds = [b"protocol_registry"],
        bump
    )]
    pub protocol_registry: Account<'info, ProtocolRegistry>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositFunds<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        init,
        payer = user,
        space = 8 + 32 + 8 + 8 + 4 + (4 + 4 + 8 + 8 + 8) * 3 + 1,
        seeds = [b"user_position", user.key().as_ref()],
        bump
    )]
    pub user_position: Account<'info, UserPosition>,

    #[account(
        mut,
        token::mint = jlp_mint.key(),
        token::authority = user
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump,
        token::mint = jlp_mint.key(),
        token::authority = protocol_registry
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        seeds = [b"protocol_registry"],
        bump
    )]
    pub protocol_registry: Account<'info, ProtocolRegistry>,

    #[account(address = Pubkey::from_str(JLP_MINT_ADDRESS).unwrap())]
    pub jlp_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawFunds<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        constraint = user_position.owner == user.key() @ YieldAggregatorError::Unauthorized,
        seeds = [b"user_position", user.key().as_ref()],
        bump
    )]
    pub user_position: Account<'info, UserPosition>,

    #[account(
        mut,
        token::mint = jlp_mint.key(),
        token::authority = user
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump,
        token::mint = jlp_mint.key(),
        token::authority = protocol_registry
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        seeds = [b"protocol_registry"],
        bump
    )]
    pub protocol_registry: Account<'info, ProtocolRegistry>,

    #[account(address = Pubkey::from_str(JLP_MINT_ADDRESS).unwrap())]
    pub jlp_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        constraint = user_position.owner == user.key() @ YieldAggregatorError::Unauthorized,
        seeds = [b"user_position", user.key().as_ref()],
        bump
    )]
    pub user_position: Account<'info, UserPosition>,

    #[account(
        seeds = [b"protocol_registry"],
        bump
    )]
    pub protocol_registry: Account<'info, ProtocolRegistry>,
}

#[derive(Accounts)]
pub struct UpdateStrategies<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        constraint = protocol_registry.admin == admin.key() @ YieldAggregatorError::Unauthorized,
        seeds = [b"protocol_registry"],
        bump
    )]
    pub protocol_registry: Account<'info, ProtocolRegistry>,
}