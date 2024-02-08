#include <thread>
#include <mutex>

thread_local static int COUNTER = 0;

static std::recursive_mutex backtrace_mutex;

extern "C"
{

    /// @brief  Reentrancy guard counter plus 1.
    /// @return The new value of the counter.
    int reentrancy_guard_counter_add()
    {
        COUNTER += 1;

        return COUNTER;
    }

    /// @brief Reentrancy guard counter sub 1.
    /// @return The new value of the counter.
    int reentrancy_guard_counter_sub()
    {
        COUNTER -= 1;

        return COUNTER;
    }

    /// @brief locks the backtrace mutex, blocks if the mutex is not available
    void backtrace_mutex_lock()
    {
        backtrace_mutex.lock();
    }

    /// @brief unlocks the backtrace mutex.
    void backtrace_mutex_unlock()
    {
        backtrace_mutex.unlock();
    }

    void *recursive_mutex_create()
    {
        return new std::recursive_mutex();
    }

    void recursive_mutex_destroy(void *mutex)
    {
        delete reinterpret_cast<std::recursive_mutex *>(mutex);
    }

    void recursive_mutex_lock(void *mutex)
    {
        reinterpret_cast<std::recursive_mutex *>(mutex)->lock();
    }

    void recursive_mutex_unlock(void *mutex)
    {
        reinterpret_cast<std::recursive_mutex *>(mutex)->unlock();
    }
}