module simple_market::coins {
    public struct BaseCoin has store, copy, drop {}
    public struct QuoteCoin has store, copy, drop {}
}

module simple_market::market_setup {
    use std::option;
    use std::signer;
    use std::string;
    use std::vector;
    use aptos_framework::coin;
    use aptos_framework::managed_coin;
    use aptos_std::type_info;
    use aptos_std::type_info::TypeInfo;

    use aptos_experimental::market;
    use aptos_experimental::market_types;
    use aptos_experimental::order_book_types::{OrderIdType, TriggerCondition};
    use simple_market::vault;
    use simple_market::coins::{BaseCoin, QuoteCoin};

    const ECONFLICTING_MARKET: u64 = 1;
    const EINVALID_ORDER_SIZE: u64 = 2;
    const EMARKET_NOT_FOUND: u64 = 3;
    const EPRICE_OVERFLOW: u64 = 5;
    const EORDER_NOT_FOUND: u64 = 6;

    struct OrderMetadata has store, copy, drop {
        market: address,
    }

    struct MarketStore has key {
        base: TypeInfo,
        quote: TypeInfo,
        market: market::Market<OrderMetadata>,
    }

    fun ensure_coin_initialized<CoinType: copy + drop + store>(
        authority: &signer,
        name: vector<u8>,
        symbol: vector<u8>,
        decimals: u8,
    ) {
        if (!coin::is_coin_initialized<CoinType>()) {
            managed_coin::initialize<CoinType>(authority, name, symbol, decimals, true);
        };
        if (!coin::is_account_registered<CoinType>(signer::address_of(authority))) {
            coin::register<CoinType>(authority);
        };
    }

    fun ensure_registered<CoinType: copy + drop + store>(account: &signer) {
        if (!coin::is_account_registered<CoinType>(signer::address_of(account))) {
            coin::register<CoinType>(account);
        };
    }

    public entry fun register_trader(trader: &signer) {
        ensure_registered<BaseCoin>(trader);
        ensure_registered<QuoteCoin>(trader);
    }

    public entry fun mint_to_trader(
        admin: &signer,
        trader: address,
        base_amount: u64,
        quote_amount: u64,
    ) {
        if (base_amount > 0) {
            managed_coin::mint<BaseCoin>(admin, trader, base_amount);
        };

        if (quote_amount > 0) {
            managed_coin::mint<QuoteCoin>(admin, trader, quote_amount);
        };
    }

    public entry fun create_market(
        admin: &signer,
        market_signer: &signer,
        allow_self_matching: bool,
        allow_events_emission: bool,
        pre_cancellation_window_secs: u64,
    ) {
        ensure_coin_initialized<BaseCoin>(admin, b"Base Test Coin", b"BASE", 6);
        ensure_coin_initialized<QuoteCoin>(admin, b"Quote Test Coin", b"QUOTE", 6);

        ensure_registered<BaseCoin>(market_signer);
        ensure_registered<QuoteCoin>(market_signer);

        let config = market::new_market_config(
            allow_self_matching,
            allow_events_emission,
            pre_cancellation_window_secs,
        );
        let new_market = market::new_market<OrderMetadata>(admin, market_signer, config);

        let market_address = signer::address_of(market_signer);
        assert!(!exists<MarketStore>(market_address), ECONFLICTING_MARKET);

        move_to(
            market_signer,
            MarketStore {
                base: type_info::type_of<BaseCoin>(),
                quote: type_info::type_of<QuoteCoin>(),
                market: new_market,
            },
        );
        vault::initialize(market_signer);
    }

    public entry fun place_limit_order(
        trader: &signer,
        market_signer: &signer,
        limit_price: u64,
        size: u64,
        is_bid: bool,
    ) acquires MarketStore {
        let market_address = signer::address_of(market_signer);
        assert!(exists<MarketStore>(market_address), EMARKET_NOT_FOUND);
        let market_store = borrow_global_mut<MarketStore>(market_address);
        place_limit_order_internal(
            market_address,
            market_store,
            trader,
            limit_price,
            size,
            is_bid,
            option::none<u64>(),
        );
    }

    public entry fun place_limit_order_with_client_id(
        trader: &signer,
        market_signer: &signer,
        limit_price: u64,
        size: u64,
        is_bid: bool,
        client_order_id: u64,
    ) acquires MarketStore {
        let market_address = signer::address_of(market_signer);
        assert!(exists<MarketStore>(market_address), EMARKET_NOT_FOUND);
        let market_store = borrow_global_mut<MarketStore>(market_address);
        place_limit_order_internal(
            market_address,
            market_store,
            trader,
            limit_price,
            size,
            is_bid,
            option::some(client_order_id),
        );
    }

    public entry fun cancel_order_by_client_id(
        trader: &signer,
        market_signer: &signer,
        client_order_id: u64,
    ) acquires MarketStore {
        let market_address = signer::address_of(market_signer);
        assert!(exists<MarketStore>(market_address), EMARKET_NOT_FOUND);
        let market_store = borrow_global_mut<MarketStore>(market_address);
        let callbacks = new_demo_callbacks();
        market::cancel_order_with_client_id(
            &mut market_store.market,
            trader,
            client_order_id,
            &callbacks,
        );
    }

    public entry fun decrease_order_size_by_client_id(
        trader: &signer,
        market_signer: &signer,
        client_order_id: u64,
        size_delta: u64,
    ) acquires MarketStore {
        assert!(size_delta > 0, EINVALID_ORDER_SIZE);
        let market_address = signer::address_of(market_signer);
        assert!(exists<MarketStore>(market_address), EMARKET_NOT_FOUND);
        let market_store = borrow_global_mut<MarketStore>(market_address);
        let market = &mut market_store.market;
        let trader_addr = signer::address_of(trader);
        let order_id_option = market
            .get_order_book()
            .get_order_id_by_client_id(trader_addr, client_order_id);
        assert!(option::is_some(&order_id_option), EORDER_NOT_FOUND);
        let order_id = option::destroy_some(order_id_option);
        let callbacks = new_demo_callbacks();
        market::decrease_order_size(
            market,
            trader,
            order_id,
            size_delta,
            &callbacks,
        );
    }

    public entry fun replace_order_by_client_id(
        trader: &signer,
        market_signer: &signer,
        client_order_id: u64,
        limit_price: u64,
        size: u64,
        is_bid: bool,
    ) acquires MarketStore {
        let market_address = signer::address_of(market_signer);
        assert!(exists<MarketStore>(market_address), EMARKET_NOT_FOUND);
        let market_store = borrow_global_mut<MarketStore>(market_address);
        let callbacks = new_demo_callbacks();
        market::cancel_order_with_client_id(
            &mut market_store.market,
            trader,
            client_order_id,
            &callbacks,
        );
        place_limit_order_internal(
            market_address,
            market_store,
            trader,
            limit_price,
            size,
            is_bid,
            option::some(client_order_id),
        );
    }

    fun place_limit_order_internal(
        market_address: address,
        market_store: &mut MarketStore,
        trader: &signer,
        limit_price: u64,
        size: u64,
        is_bid: bool,
        client_order_id: option::Option<u64>,
    ) {
        assert!(size > 0, EINVALID_ORDER_SIZE);

        ensure_registered<BaseCoin>(trader);
        ensure_registered<QuoteCoin>(trader);

        reserve_order_funds(market_address, trader, limit_price, size, is_bid);
        let time_in_force = market_types::good_till_cancelled();
        let callbacks = new_demo_callbacks();
        let metadata = OrderMetadata { market: market_address };

        let _ = market::place_limit_order<OrderMetadata>(
            &mut market_store.market,
            trader,
            limit_price,
            size,
            is_bid,
            time_in_force,
            option::none<TriggerCondition>(),
            metadata,
            client_order_id,
            10,
            false,
            &callbacks,
        );
    }

    fun new_demo_callbacks(): market_types::MarketClearinghouseCallbacks<OrderMetadata> {
        market_types::new_market_clearinghouse_callbacks<OrderMetadata>(
            settle_trade_callback,
            validate_order_callback,
            place_maker_order_callback,
            cleanup_order_callback,
            decrease_order_size_callback,
            metadata_bytes_callback,
        )
    }

    fun settle_trade_callback(
        taker: address,
        _taker_order_id: OrderIdType,
        maker: address,
        _maker_order_id: OrderIdType,
        _fill_id: u64,
        is_taker_long: bool,
        price: u64,
        size: u64,
        taker_metadata: OrderMetadata,
        _maker_metadata: OrderMetadata,
    ): market_types::SettleTradeResult {
        if (size == 0) {
            return market_types::new_settle_trade_result(
                0,
                option::none<string::String>(),
                option::none<string::String>(),
            );
        };

        let market_addr = taker_metadata.market;
        let quote_amount = compute_quote_amount(price, size);

        if (is_taker_long) {
            // Taker buys base: maker sells base for quote.
            let quote_payment = vault::withdraw_quote(market_addr, taker, quote_amount);
            let base_delivery = vault::withdraw_base(market_addr, maker, size);

            coin::deposit<QuoteCoin>(maker, quote_payment);
            coin::deposit<BaseCoin>(taker, base_delivery);
        } else {
            // Taker sells base: maker buys base with quote.
            let base_payment = vault::withdraw_base(market_addr, taker, size);
            let quote_delivery = vault::withdraw_quote(market_addr, maker, quote_amount);

            coin::deposit<BaseCoin>(maker, base_payment);
            coin::deposit<QuoteCoin>(taker, quote_delivery);
        };

        market_types::new_settle_trade_result(
            size,
            option::none<string::String>(),
            option::none<string::String>(),
        )
    }

    fun validate_order_callback(
        _account: address,
        _order_id: OrderIdType,
        _is_taker: bool,
        _is_bid: bool,
        _price: option::Option<u64>,
        _size: u64,
        _metadata: OrderMetadata,
    ): bool {
        true
    }

    fun place_maker_order_callback(
        _account: address,
        _order_id: OrderIdType,
        _is_bid: bool,
        _price: u64,
        _size: u64,
        _metadata: OrderMetadata,
    ) {}

    fun cleanup_order_callback(
        _account: address,
        _order_id: OrderIdType,
        _is_bid: bool,
        _remaining_size: u64,
    ) {}

    fun decrease_order_size_callback(
        _account: address,
        _order_id: OrderIdType,
        _is_bid: bool,
        _price: u64,
        _size: u64,
    ) {}

    fun metadata_bytes_callback(_metadata: OrderMetadata): vector<u8> {
        vector::empty<u8>()
    }

    fun reserve_order_funds(
        market_addr: address,
        trader: &signer,
        limit_price: u64,
        size: u64,
        is_bid: bool,
    ) {
        let trader_addr = signer::address_of(trader);

        if (is_bid) {
            let quote_amount = compute_quote_amount(limit_price, size);
            if (quote_amount == 0) {
                return;
            };
            let quote_coin = coin::withdraw<QuoteCoin>(trader, quote_amount);
            vault::deposit_quote(market_addr, trader_addr, quote_coin);
        } else {
            let base_amount = size;
            if (base_amount == 0) {
                return;
            };
            let base_coin = coin::withdraw<BaseCoin>(trader, base_amount);
            vault::deposit_base(market_addr, trader_addr, base_coin);
        };
    }

fun compute_quote_amount(price: u64, size: u64): u64 {
    if (size == 0 || price == 0) {
        return 0;
    };
    let max_price = (0xffffffffffffffff / size);
    assert!(price <= max_price, EPRICE_OVERFLOW);
    price * size
}
}

module simple_market::vault {
    use std::signer;
    use aptos_framework::coin;
    use aptos_std::table;
    use aptos_std::table::Table;
    use simple_market::coins::{BaseCoin, QuoteCoin};

    const EINSUFFICIENT_ESCROW: u64 = 4;

    struct VaultStore has key {
        vaults: Table<address, TraderVault>,
    }

    struct TraderVault has store {
        base: coin::Coin<BaseCoin>,
        quote: coin::Coin<QuoteCoin>,
    }

    public fun initialize(market_signer: &signer) {
        let market_addr = signer::address_of(market_signer);
        if (!exists<VaultStore>(market_addr)) {
            move_to(
                market_signer,
                VaultStore {
                    vaults: table::new(),
                },
            );
        };
    }

    public fun deposit_base(
        market_addr: address,
        trader: address,
        coin_to_add: coin::Coin<BaseCoin>,
    ) acquires VaultStore {
        let vault_store = borrow_global_mut<VaultStore>(market_addr);
        if (!table::contains(&vault_store.vaults, trader)) {
            table::add(
                &mut vault_store.vaults,
                trader,
                TraderVault {
                    base: coin::zero<BaseCoin>(),
                    quote: coin::zero<QuoteCoin>(),
                },
            );
        };
        let vault = table::borrow_mut(&mut vault_store.vaults, trader);
        coin::merge(&mut vault.base, coin_to_add);
    }

    public fun deposit_quote(
        market_addr: address,
        trader: address,
        coin_to_add: coin::Coin<QuoteCoin>,
    ) acquires VaultStore {
        let vault_store = borrow_global_mut<VaultStore>(market_addr);
        if (!table::contains(&vault_store.vaults, trader)) {
            table::add(
                &mut vault_store.vaults,
                trader,
                TraderVault {
                    base: coin::zero<BaseCoin>(),
                    quote: coin::zero<QuoteCoin>(),
                },
            );
        };
        let vault = table::borrow_mut(&mut vault_store.vaults, trader);
        coin::merge(&mut vault.quote, coin_to_add);
    }

    public fun withdraw_base(
        market_addr: address,
        trader: address,
        amount: u64,
    ): coin::Coin<BaseCoin> acquires VaultStore {
        if (amount == 0) {
            return coin::zero<BaseCoin>();
        };
        let vault_store = borrow_global_mut<VaultStore>(market_addr);
        if (!table::contains(&vault_store.vaults, trader)) {
            table::add(
                &mut vault_store.vaults,
                trader,
                TraderVault {
                    base: coin::zero<BaseCoin>(),
                    quote: coin::zero<QuoteCoin>(),
                },
            );
        };
        let vault = table::borrow_mut(&mut vault_store.vaults, trader);
        assert!(coin::value(&vault.base) >= amount, EINSUFFICIENT_ESCROW);
        coin::extract(&mut vault.base, amount)
    }

    public fun withdraw_quote(
        market_addr: address,
        trader: address,
        amount: u64,
    ): coin::Coin<QuoteCoin> acquires VaultStore {
        if (amount == 0) {
            return coin::zero<QuoteCoin>();
        };
        let vault_store = borrow_global_mut<VaultStore>(market_addr);
        if (!table::contains(&vault_store.vaults, trader)) {
            table::add(
                &mut vault_store.vaults,
                trader,
                TraderVault {
                    base: coin::zero<BaseCoin>(),
                    quote: coin::zero<QuoteCoin>(),
                },
            );
        };
        let vault = table::borrow_mut(&mut vault_store.vaults, trader);
        assert!(coin::value(&vault.quote) >= amount, EINSUFFICIENT_ESCROW);
        coin::extract(&mut vault.quote, amount)
    }
}
